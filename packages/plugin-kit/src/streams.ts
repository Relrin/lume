/**
 * A stream of bytes between a view and something its plugin's code has open: a file, a
 * program's input or output, a connection.
 *
 * The plugin's code hands the view a grant (`lume.streams.grantToView`), the view opens
 * it, and from then on the bytes go between the application and the view without
 * passing through the plugin's code. The stream has a port of its own. What arrives on
 * it is `{ d: ArrayBuffer }`, a piece, and `{ end: true, ok }` when the stream is over;
 * what is sent is `{ ack: n }`, that `n` bytes were taken, which lets as many more
 * come, `{ w: ArrayBuffer }`, bytes to write, `{ fin: true }`, that no more will be
 * written, and `{ close: true }`.
 *
 * No more than about a megabyte is ever on its way to a view: pieces a view has not
 * said it took are not followed by more. A piece counts as taken when the view's
 * listener has returned, or, where it returns a promise, when that has settled, so a
 * view that is slow holds the stream back rather than filling up.
 */

/** The most that is written in one message; longer writes are sent in parts. */
export const WRITE_MAX = 64 * 1024;

/** How often, at most, a view says how much it has taken, in milliseconds. */
export const ACK_EVERY_MS = 4;

/** The part of a port a stream uses. */
export interface StreamPortLike {
  postMessage(message: unknown, transfer?: Transferable[]): void;
  onmessage: ((event: { data: unknown }) => void) | null;
  close(): void;
}

/** What a stream needs of time, so tests can stand it in. */
export interface StreamClock {
  now(): number;
  later(task: () => void, ms: number): void;
}

const CLOCK: StreamClock = {
  now: () => performance.now(),
  later: (task, ms) => void setTimeout(task, ms),
};

/** How a stream ended: whether all of it came, and how many bytes did. */
export interface StreamEnd {
  readonly ok: boolean;
  readonly bytes: number;
}

/** Takes the pieces of a stream as they come. */
export type PieceListener = (piece: Uint8Array) => void | Promise<void>;

/** One open stream, from the view's side. */
export class ViewStream {
  /** Whether bytes arrive on it. */
  readonly readable: boolean;
  /** Whether bytes can be written to it. */
  readonly writable: boolean;
  /** Settles when the stream is over, with how it ended. */
  readonly ended: Promise<StreamEnd>;

  readonly #port: StreamPortLike;
  readonly #clock: StreamClock;
  #listener: PieceListener | null = null;
  /** Pieces that came before anyone listened. */
  #held: Uint8Array[] = [];
  #bytes = 0;
  #closed = false;
  #finish: (end: StreamEnd) => void = () => undefined;
  /** Bytes taken that the application has not been told of yet. */
  #owed = 0;
  #ackedAt = -Infinity;
  #ackWaiting = false;
  /** The end, where it came while pieces were still held. */
  #endHeld: boolean | null = null;

  constructor(
    port: StreamPortLike,
    ways: { readonly readable: boolean; readonly writable: boolean },
    clock: StreamClock = CLOCK,
  ) {
    this.#port = port;
    this.#clock = clock;
    this.readable = ways.readable;
    this.writable = ways.writable;
    this.ended = new Promise((resolve) => {
      this.#finish = resolve;
    });
    port.onmessage = (event) => this.#receive(event.data);
  }

  /** How many bytes have arrived so far. */
  get bytes(): number {
    return this.#bytes;
  }

  get closed(): boolean {
    return this.#closed;
  }

  /**
   * Hands every piece of the stream to `listener`, those that came already first. One
   * listener at a time: a second takes the first one's place.
   */
  onData(listener: PieceListener): void {
    this.#listener = listener;
    const held = this.#held;
    this.#held = [];
    for (const piece of held) this.#give(piece);
    if (this.#endHeld !== null && this.#held.length === 0) this.#over(this.#endHeld);
  }

  #receive(data: unknown): void {
    if (this.#closed || typeof data !== "object" || data === null) return;
    const message = data as { d?: unknown; end?: unknown; ok?: unknown };
    if (message.d instanceof ArrayBuffer) {
      const piece = new Uint8Array(message.d);
      this.#bytes += piece.byteLength;
      if (this.#listener === null) this.#held.push(piece);
      else this.#give(piece);
      return;
    }
    if (message.end === true) {
      const ok = message.ok === true;
      if (this.#held.length > 0) this.#endHeld = ok;
      else this.#over(ok);
    }
  }

  #give(piece: Uint8Array): void {
    const taken = () => this.#took(piece.byteLength);
    let result: void | Promise<void>;
    try {
      result = this.#listener?.(piece);
    } catch (error) {
      taken();
      throw error;
    }
    if (result && typeof result.then === "function") result.then(taken, taken);
    else taken();
  }

  /** Says the bytes were taken: at once after a quiet spell, else together, soon. */
  #took(count: number): void {
    if (this.#closed) return;
    this.#owed += count;
    if (this.#ackWaiting) return;
    const since = this.#clock.now() - this.#ackedAt;
    if (since >= ACK_EVERY_MS) {
      this.#ack();
      return;
    }
    this.#ackWaiting = true;
    this.#clock.later(() => {
      this.#ackWaiting = false;
      this.#ack();
    }, ACK_EVERY_MS - since);
  }

  #ack(): void {
    if (this.#closed || this.#owed === 0) return;
    const owed = this.#owed;
    this.#owed = 0;
    this.#ackedAt = this.#clock.now();
    this.#post({ ack: owed });
  }

  #post(message: unknown, transfer?: Transferable[]): void {
    try {
      this.#port.postMessage(message, transfer ?? []);
    } catch {
      this.#over(false);
    }
  }

  #over(ok: boolean): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#port.onmessage = null;
    this.#port.close();
    this.#finish({ ok, bytes: this.#bytes });
  }

  /**
   * Writes bytes, or text as UTF-8, to the stream. A long write is sent in parts. The
   * application takes a few hundred parts a second from one stream: a view that writes
   * many small pieces should gather them first.
   */
  write(data: Uint8Array | string): void {
    if (this.#closed) throw new Error("the stream is closed");
    if (!this.writable) throw new Error("the stream cannot be written to");
    const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
    for (let at = 0; at < bytes.byteLength; at += WRITE_MAX) {
      // A buffer of the part's own, which then leaves this page.
      const buffer = bytes.slice(at, at + WRITE_MAX).buffer;
      this.#post({ w: buffer }, [buffer]);
      if (this.#closed) return;
    }
  }

  /** Says nothing more will be written, as closing a program's input does. */
  finish(): void {
    if (this.#closed || !this.writable) return;
    this.#post({ fin: true });
  }

  /** Ends the stream from the view's side. */
  close(): void {
    if (this.#closed) return;
    this.#post({ close: true });
    this.#over(false);
  }
}
