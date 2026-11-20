/**
 * Carrying a plugin's stream of bytes between the core and one of the plugin's views.
 *
 * A view that was handed a grant opens it, and from then on the stream's bytes go
 * between the core and the view's frame without passing through the plugin's code: the
 * core sends them on the page's carrier, and the page hands each piece to the frame on
 * a port of the stream's own, as a buffer that changes hands rather than being copied.
 *
 * What a frame is sent on a stream's port: `{ d: ArrayBuffer }`, a piece of the
 * stream, and `{ end: true, ok }` when it is over. What it may send: `{ ack: n }`, that
 * it has taken `n` bytes, which lets as many more come; `{ w: ArrayBuffer }`, bytes to
 * write; `{ fin: true }`, that it will write no more; `{ close: true }`.
 *
 * The page does this work between a terminal's frames, so it is kept small and
 * bounded. No more than one window of a stream is ever on its way to a frame or waiting
 * in it: a frame that does not say it took what it was sent is sent no more. What is
 * handed over in one go is limited in time, and what a stream may carry in a second is
 * limited too, so a busy stream takes a known share of the page's time and no more. A
 * frame that floods a stream's port loses the stream.
 *
 * None of this is React's: a stream is opened and carried by plain objects, and nothing
 * renders because of it.
 */

import { Rate } from "../plugins/frame/bridge.ts";
import type { Pipe } from "./client.ts";

/** How many bytes of a stream may be on their way to a frame, or waiting in it. */
export const STREAM_WINDOW = 1024 * 1024;

/**
 * How many bytes a stream carries to its frame in a second, at most, and how many at
 * once after a quiet spell. A stream read as fast as it comes goes at some 60 to 80
 * MiB a second on the machine this was measured on, held to that by the window above
 * and the way there and back; the window's page then still answers within two
 * milliseconds 95 times in a hundred. The limit is above that speed, so that a way
 * that becomes faster does not take more of the page's time than was measured.
 */
export const STREAM_BYTES_PER_SECOND = 128 * 1024 * 1024;
export const STREAM_BYTES_BURST = 4 * 1024 * 1024;

/** How long the page hands pieces over in one go before it lets other work in. */
export const RELAY_SLICE_MS = 2;

/** The most a frame writes to a stream in one message. */
export const WRITE_MAX = 64 * 1024;

/** How many messages a second a frame may send on a stream's port, and how many at once. */
const PORT_RATE_PER_SECOND = 400;
const PORT_RATE_BURST = 800;

/** How many streams one frame may have open at once. */
export const STREAMS_PER_FRAME = 16;

/** The part of a port a relay uses, so tests can stand one in. */
export interface StreamPort {
  postMessage(message: unknown, transfer?: Transferable[]): void;
  onmessage: ((event: MessageEvent) => void) | null;
  close(): void;
}

/** What a relay needs from its surroundings. */
export interface RelayClock {
  now(): number;
  /** Runs `task` after the page's other work has had its turn. */
  later(task: () => void, ms: number): void;
  /** Runs `task` once what the page is doing right now is done. */
  soon(task: () => void): void;
}

const CLOCK: RelayClock = {
  now: () => performance.now(),
  later: (task, ms) => void setTimeout(task, ms),
  soon: (task) => queueMicrotask(task),
};

/** One stream between the core and a frame. */
export class StreamRelay {
  readonly #pipe: Pipe;
  readonly #port: StreamPort;
  readonly #clock: RelayClock;
  readonly #rate: Rate;
  readonly #gone: () => void;
  /** Pieces from the core that have not been handed to the frame yet. */
  #queue: Uint8Array[] = [];
  #draining = false;
  #closed = false;
  /** How the stream ended, once it has: told to the frame after the last piece. */
  #ended: boolean | null = null;
  /** The bytes a second the stream has left, and when that was last worked out. */
  #allowance = STREAM_BYTES_BURST;
  #allowanceAt: number;
  /** Bytes handed to the frame that it has not said it took. */
  #unacked = 0;
  /**
   * When the stream began handing pieces over in the page's present turn. Pieces that
   * arrive together arrive one call after another, and share the one short while.
   */
  #turnBegan: number | null = null;

  constructor(pipe: Pipe, port: StreamPort, gone: () => void, clock: RelayClock = CLOCK) {
    this.#pipe = pipe;
    this.#port = port;
    this.#clock = clock;
    this.#gone = gone;
    this.#allowanceAt = clock.now();
    this.#rate = new Rate(PORT_RATE_PER_SECOND, PORT_RATE_BURST, clock.now());
    port.onmessage = (event) => this.#fromFrame(event.data);
    pipe.start(
      (bytes) => {
        if (this.#closed) return;
        this.#queue.push(bytes);
        this.#drain();
      },
      (ok) => {
        if (this.#closed) return;
        this.#ended = ok;
        this.#drain();
      },
    );
  }

  get closed(): boolean {
    return this.#closed;
  }

  /** Hands the frame what waits, for a short while and within the stream's rate. */
  #drain(): void {
    if (this.#draining || this.#closed) return;
    this.#draining = true;
    const began = this.#clock.now();
    if (this.#turnBegan === null) {
      this.#turnBegan = began;
      this.#clock.soon(() => {
        this.#turnBegan = null;
      });
    }
    const turnBegan = this.#turnBegan;
    const elapsed = Math.max(0, began - this.#allowanceAt);
    this.#allowanceAt = began;
    this.#allowance = Math.min(
      STREAM_BYTES_BURST,
      this.#allowance + (elapsed / 1000) * STREAM_BYTES_PER_SECOND,
    );
    while (this.#queue.length > 0) {
      if (this.#allowance <= 0) {
        // Until the stream has earned back what it is over by, and a byte more.
        this.#again(((1 - this.#allowance) / STREAM_BYTES_PER_SECOND) * 1000);
        return;
      }
      if (this.#clock.now() - turnBegan >= RELAY_SLICE_MS) {
        this.#again(0);
        return;
      }
      const bytes = this.#queue.shift();
      if (bytes === undefined) break;
      // The carrier's bytes are a view into what it received; the frame is given a
      // buffer of its own, which then leaves this page.
      const buffer = bytes.slice().buffer;
      this.#allowance -= bytes.length;
      this.#unacked += bytes.length;
      try {
        this.#port.postMessage({ d: buffer }, [buffer]);
      } catch {
        this.close();
        return;
      }
    }
    this.#draining = false;
    if (this.#ended !== null) {
      try {
        this.#port.postMessage({ end: true, ok: this.#ended });
      } catch {
        // The frame has gone; so has the stream.
      }
      this.close();
    }
  }

  #again(ms: number): void {
    this.#clock.later(() => {
      this.#draining = false;
      this.#turnBegan = null;
      this.#drain();
    }, Math.ceil(ms));
  }

  #fromFrame(data: unknown): void {
    if (this.#closed) return;
    if (this.#rate.take(this.#clock.now()) !== "ok") {
      this.close();
      return;
    }
    if (typeof data !== "object" || data === null) {
      this.close();
      return;
    }
    const message = data as Record<string, unknown>;
    if (typeof message["ack"] === "number") {
      const taken = message["ack"];
      // A frame cannot take more than it was handed.
      if (!Number.isSafeInteger(taken) || taken < 0 || taken > this.#unacked) {
        this.close();
        return;
      }
      this.#unacked -= taken;
      this.#pipe.consumed(taken);
      return;
    }
    if (message["w"] instanceof ArrayBuffer) {
      const bytes = new Uint8Array(message["w"]);
      if (bytes.length > WRITE_MAX || !this.#pipe.writable) {
        this.close();
        return;
      }
      this.#pipe.write(bytes);
      return;
    }
    if (message["fin"] === true) {
      if (this.#pipe.writable) this.#pipe.finish();
      return;
    }
    // Anything else, a wish to close among it, ends the stream.
    this.close();
  }

  /** Ends the stream: the core is told, and the frame's port is closed. */
  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#queue = [];
    this.#port.onmessage = null;
    this.#port.close();
    this.#pipe.close();
    this.#gone();
  }
}

/** The streams one frame has open. */
export class FrameStreams {
  readonly #open = new Set<StreamRelay>();
  #closed = false;

  constructor(
    /** Opens a grant as a stream of the page's carrier. */
    private readonly openPipe: (grant: string) => Promise<Pipe>,
    private readonly clock: RelayClock = CLOCK,
  ) {}

  get count(): number {
    return this.#open.size;
  }

  /**
   * Opens `grant` for the frame, which is given `port` as its end of the stream. Says
   * whether the stream is read, written or both.
   */
  async open(
    grant: string,
    makePorts: () => { readonly mine: StreamPort; readonly theirs: Transferable },
  ): Promise<{
    readonly port: Transferable;
    readonly readable: boolean;
    readonly writable: boolean;
  }> {
    if (this.#closed) throw new Error("the view has closed");
    if (this.#open.size >= STREAMS_PER_FRAME) {
      throw Object.assign(new Error(`a view has ${STREAMS_PER_FRAME} streams open at most`), {
        code: "busy",
      });
    }
    const pipe = await this.openPipe(grant);
    if (this.#closed) {
      pipe.close();
      throw new Error("the view has closed");
    }
    const ports = makePorts();
    const relay: StreamRelay = new StreamRelay(
      pipe,
      ports.mine,
      () => this.#open.delete(relay),
      this.clock,
    );
    if (!relay.closed) this.#open.add(relay);
    return { port: ports.theirs, readable: pipe.readable, writable: pipe.writable };
  }

  /** Ends every stream: the frame has gone. */
  close(): void {
    this.#closed = true;
    for (const relay of [...this.#open]) relay.close();
  }
}
