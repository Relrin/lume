/**
 * The page's end of the session transport.
 *
 * It starts, lists and ends sessions through the core's commands, and attaches to them:
 * an attachment is a stream of frames from the session and a way to send input, sizes,
 * facts and credit back. Many attachments share one carrier.
 *
 * Two rules keep the bytes right. Frames to the core leave in one request at a time,
 * because requests in flight together may overtake each other and input must not. And
 * output is granted in windows as it is consumed, so what waits in this page for a
 * terminal to take it is never more than one window per stream.
 */

import type { Carrier, CarrierOpened } from "./carrier.ts";
import { Granter } from "./credit.ts";
import { FrameDecoder } from "./decoder.ts";
import {
  encodeCredit,
  encodeDetach,
  encodeRehydrate,
  encodeFacts,
  encodeInput,
  encodeResize,
  type HostFrame,
} from "./frames.ts";
import {
  type BackendCaps,
  capsFromWire,
  type CellPx,
  type ClientFacts,
  CommandError,
  factsToWire,
  type Launcher,
  sessionFromWire,
  type SessionInfo,
  type SpawnRequest,
  spawnToWire,
  type TermSize,
  toCommandError,
} from "./types.ts";

/** A frame of one attachment. */
export type StreamFrame = Exclude<HostFrame, { kind: "control" }>;

/** How a new attachment is brought up to date before its output. */
export type HydrateMode = "none" | "snapshot" | "vt_replay";

/** The state an attachment starts from: a saved terminal, or output that rebuilds one. */
export type HydrateFrame = Extract<StreamFrame, { kind: "hydrate" }>;

/** An open attachment: everything that connects a view to its session. */
export interface Attachment {
  /** The terminal's size when the attachment opened. */
  readonly size: TermSize;
  /** Whether the view's terminal should pull lines out of scrollback when it grows. */
  readonly resizePullsScrollback: boolean;
  /** Whether a hydrate frame precedes the output. */
  readonly hydrate: HydrateMode;
  readonly caps: BackendCaps;
  /** What the session runs, where the application started it and so knows. */
  readonly launcher?: Launcher;
  /** The profile it was started from, if it was. */
  readonly profile?: string;

  /**
   * The state the session's screen was in when the attachment opened, for a view to
   * start from before it takes any output; `undefined` when there is none. Call it
   * before {@link start}: the frame is taken out of the flow.
   */
  hydrated(): Promise<HydrateFrame | undefined>;
  /**
   * Starts the flow. `receive` gets every frame in order, beginning with any that arrived
   * while the attachment was being made. No output is sent before this is called.
   */
  start(receive: (frame: StreamFrame) => void): void;
  /** Sends bytes to the session's program, as if typed. */
  input(bytes: Uint8Array): void;
  /** Asks for the terminal to be resized. The answer is a `resized` frame. */
  resize(size: TermSize, cellPx: CellPx): void;
  /** Tells the session how the view has changed. */
  facts(facts: ClientFacts): void;
  /** Reports that `bytes` of received output have been processed, which lets more come. */
  consumed(bytes: number): void;
  /**
   * Asks for the state to start again from, as of this point of the output. It arrives
   * as a `hydrate` frame in its place among the others, from a backend whose caps say it
   * can hydrate.
   */
  rehydrate(): void;
  /** Ends the attachment. The session carries on. */
  detach(): void;
}

interface Stream {
  readonly granter: Granter;
  /** Frames kept until the view is ready for them; `null` once it is. */
  waiting: StreamFrame[] | null;
  receive: ((frame: StreamFrame) => void) | null;
  /** Whoever waits for the state the attachment starts from, until it arrives. */
  onHydrate: ((frame: HydrateFrame | undefined) => void) | null;
}

interface WireAttached {
  size: TermSize;
  resize_pulls_scrollback: boolean;
  hydrate: HydrateMode;
  caps: unknown;
  launcher?: Launcher;
  profile?: string;
}

export interface TransportOptions {
  /**
   * Called once if the transport can no longer be used: the core sent something that is
   * not a valid stream of frames, or refused this page's carrier.
   */
  onFailure?(error: Error): void;
  /** Called when something a view asked for could not be done. Defaults to the console. */
  onError?(error: Error): void;
  /**
   * Called with each control message the core sends on the carrier: a request to the
   * page, to be answered with {@link TransportClient.answer}. A client that is given no
   * handler treats a control message as a fault of the transport.
   */
  onControl?(message: unknown): void;
}

export class TransportClient {
  readonly #carrier: Carrier;
  readonly #opened: CarrierOpened;
  readonly #options: TransportOptions;
  readonly #decoder: FrameDecoder;
  readonly #streams = new Map<number, Stream>();
  #nextStream = 1;
  #outbox: Uint8Array[] = [];
  #flushQueued = false;
  #sending = false;
  #failed: Error | undefined;

  private constructor(carrier: Carrier, opened: CarrierOpened, options: TransportOptions) {
    this.#carrier = carrier;
    this.#opened = opened;
    this.#options = options;
    this.#decoder = new FrameDecoder(opened.limits);
  }

  /** Opens `carrier` and returns a client on it. */
  static async open(carrier: Carrier, options: TransportOptions = {}): Promise<TransportClient> {
    // Frames cannot arrive before the open is answered, but the callback must exist
    // before the client does.
    const early: Uint8Array[] = [];
    let receive = (frames: Uint8Array): void => void early.push(frames);
    const opened = await carrier.open((frames) => receive(frames));
    const client = new TransportClient(carrier, opened, options);
    receive = (frames) => client.#receive(frames);
    for (const frames of early) client.#receive(frames);
    return client;
  }

  /** The backend new sessions start on. */
  get defaultBackend(): string {
    return this.#opened.defaultBackend;
  }

  /** Starts a session and returns its id. */
  spawn(backend: string, request: SpawnRequest): Promise<string> {
    return this.command("session_spawn", { backend, spec: spawnToWire(request) });
  }

  /** Every session of every backend. */
  async list(): Promise<SessionInfo[]> {
    const listed = await this.command<unknown[]>("session_list", {});
    return listed.map(sessionFromWire);
  }

  /** Ends a session. */
  async kill(backend: string, session: string): Promise<void> {
    await this.command("session_kill", { backend, session });
  }

  /**
   * Answers a request the core sent as a control message. `id` is the request's, and
   * `result` is `{ ok: … }` with what it yielded or `{ err: { code, message } }`.
   */
  async answer(id: number, result: unknown): Promise<void> {
    await this.command("page_reply", { id, result });
  }

  /** Attaches to a session. `facts` describes the view it will be shown in. */
  async attach(backend: string, session: string, facts: ClientFacts): Promise<Attachment> {
    const id = this.#nextStream++;
    const stream: Stream = {
      granter: new Granter(this.#opened.creditWindow),
      waiting: [],
      receive: null,
      onHydrate: null,
    };
    // The stream exists before the request goes out: frames for it may arrive before
    // the answer does, and they must find it.
    this.#streams.set(id, stream);
    let attached: WireAttached;
    try {
      attached = await this.command<WireAttached>("session_attach", {
        backend,
        session,
        stream: id,
        facts: factsToWire(facts),
      });
    } catch (error) {
      this.#streams.delete(id);
      throw error;
    }

    const open = () => this.#streams.get(id) === stream;
    return {
      size: attached.size,
      resizePullsScrollback: attached.resize_pulls_scrollback,
      hydrate: attached.hydrate,
      caps: capsFromWire(attached.caps),
      ...(attached.launcher ? { launcher: attached.launcher } : {}),
      ...(typeof attached.profile === "string" ? { profile: attached.profile } : {}),
      hydrated: () => {
        const waiting = stream.waiting;
        if (attached.hydrate === "none" || !open() || !waiting) return Promise.resolve(undefined);
        const at = waiting.findIndex((frame) => frame.kind === "hydrate");
        if (at >= 0) {
          const [frame] = waiting.splice(at, 1);
          return Promise.resolve(frame as HydrateFrame);
        }
        return new Promise((resolve) => {
          stream.onHydrate = resolve;
        });
      },
      start: (receive) => {
        if (!open() || stream.receive) return;
        stream.receive = receive;
        this.#enqueue(encodeCredit(id, stream.granter.firstGrant));
        const waiting = stream.waiting ?? [];
        stream.waiting = null;
        for (const frame of waiting) this.#deliver(stream, frame);
      },
      input: (bytes) => {
        if (open()) this.#enqueue(encodeInput(id, bytes, this.#opened.limits.maxPayload));
      },
      resize: (size, cellPx) => {
        if (open()) this.#enqueue(encodeResize(id, size, cellPx));
      },
      facts: (next) => {
        if (open()) this.#enqueue(encodeFacts(id, next));
      },
      consumed: (bytes) => {
        if (!open()) return;
        const grant = stream.granter.consumed(bytes);
        if (grant > 0) this.#enqueue(encodeCredit(id, grant));
      },
      rehydrate: () => {
        if (open()) this.#enqueue(encodeRehydrate(id));
      },
      detach: () => {
        if (!open()) return;
        this.#streams.delete(id);
        stream.onHydrate?.(undefined);
        stream.onHydrate = null;
        this.#enqueue(encodeDetach(id));
      },
    };
  }

  /**
   * Calls one of the core's commands as this page's carrier. A refusal rejects with a
   * {@link CommandError}.
   */
  async command<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
    if (this.#failed) throw this.#failed;
    try {
      return await this.#carrier.command<T>(name, { carrier: this.#opened.carrier, ...args });
    } catch (thrown) {
      throw toCommandError(thrown);
    }
  }

  #receive(chunk: Uint8Array): void {
    if (this.#failed) return;
    try {
      this.#decoder.push(chunk);
      for (let frame = this.#decoder.next(); frame; frame = this.#decoder.next()) {
        if (frame.kind === "control") {
          const { onControl } = this.#options;
          if (!onControl) throw new Error("a control message arrived on the carrier");
          try {
            onControl(frame.message);
          } catch (error) {
            // A request that cannot be handled must not stop the sessions' frames.
            this.#report(error);
          }
          continue;
        }
        // A frame for no stream was on its way when the stream was detached.
        const stream = this.#streams.get(frame.stream);
        if (!stream) continue;
        if (frame.kind === "data") stream.granter.received(frame.bytes.length);
        const onHydrate = stream.onHydrate;
        if (frame.kind === "hydrate" && onHydrate) {
          stream.onHydrate = null;
          onHydrate(frame);
          continue;
        }
        if (stream.waiting) stream.waiting.push(frame);
        else this.#deliver(stream, frame);
      }
    } catch (error) {
      this.#fail(error instanceof Error ? error : new Error(String(error)));
    }
  }

  #deliver(stream: Stream, frame: StreamFrame): void {
    try {
      stream.receive?.(frame);
    } catch (error) {
      // One view's fault must not stop the frames of the others.
      this.#report(error);
    }
  }

  #enqueue(frames: Uint8Array): void {
    if (this.#failed) return;
    this.#outbox.push(frames);
    if (this.#flushQueued || this.#sending) return;
    this.#flushQueued = true;
    // Everything queued in this turn of the event loop leaves in one request.
    queueMicrotask(() => {
      this.#flushQueued = false;
      this.#flush();
    });
  }

  #flush(): void {
    if (this.#sending || this.#failed || this.#outbox.length === 0) return;
    const parts = this.#outbox;
    this.#outbox = [];
    let batch = parts[0]!;
    if (parts.length > 1) {
      batch = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
      let at = 0;
      for (const part of parts) {
        batch.set(part, at);
        at += part.length;
      }
    }
    this.#sending = true;
    this.#carrier.send(batch).then(
      () => this.#sent(),
      (thrown: unknown) => {
        const error = toCommandError(thrown);
        // A carrier the core no longer knows belongs to a page that has been replaced.
        if (error.code === "stale_carrier" || error.code === "no_carrier") this.#fail(error);
        else this.#report(error);
        this.#sent();
      },
    );
  }

  #sent(): void {
    this.#sending = false;
    this.#flush();
  }

  #fail(error: Error): void {
    if (this.#failed) return;
    this.#failed = error;
    this.#outbox = [];
    // Nothing more will arrive for anybody waiting.
    for (const stream of this.#streams.values()) stream.onHydrate?.(undefined);
    this.#streams.clear();
    this.#options.onFailure?.(error);
  }

  #report(thrown: unknown): void {
    const error = thrown instanceof Error ? thrown : new CommandError("internal", String(thrown));
    if (this.#options.onError) this.#options.onError(error);
    else console.error(error);
  }
}
