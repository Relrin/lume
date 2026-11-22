/**
 * The way between a plugin's frame and the page: one `MessagePort` per frame.
 *
 * A plugin's panels and pane views are its own pages in sandboxed frames. When a frame
 * has loaded, the page makes a channel and hands the frame one end; that end is the
 * frame's only way out. Which plugin and view a frame is was decided by the page when it
 * made the frame, from the application's own list, and is bound to the port: nothing a
 * frame sends can say it is someone else.
 *
 * What a frame may send:
 *
 * - `{ t: "ready" }`: it is set up and listening;
 * - `{ t: "call", id, name, args }`: a call to its plugin's code (`host.call`);
 * - `{ t: "state", state }`: what it wants to find again when it is shown again, as
 *   JSON of 64 KiB at most;
 * - `{ t: "key", key, code, mods }`: a key it did not use itself;
 * - `{ t: "stream", id, grant }`: it opens a stream its plugin's code let it use;
 * - `{ t: "error", message }`: it failed.
 *
 * What it is sent: `{ t: "result" | "error", id, … }` for its calls (a failure with
 * its `code`, its `message` and, where its plugin's code said more, `data`), `{ t: "opened",
 * id, port, readable, writable }` for a stream it opened, with the stream's own port,
 * `{ t: "event", event, payload }` from its plugin's code, `{ t: "tokens", tokens,
 * scheme }` when the look changes, `{ t: "focus" }` when the keyboard comes to it, and
 * `{ t: "refused", what, message }` when something it sent was not taken.
 *
 * Each frame may send 200 messages a second, with up to 400 at once after a quiet spell,
 * of 1 MiB at most each and 4 MiB a second between them, with up to 2 MiB at once. A
 * frame that keeps sending past that, or sends what is not one of the messages above,
 * is stopped.
 *
 * Nothing here touches the DOM, so all of it is tested with plain objects.
 */

/** The most a message may be, as JSON. */
export const MESSAGE_MAX = 1024 * 1024;

/** How many messages a second a frame may send, and how many at once. */
export const RATE_PER_SECOND = 200;
export const RATE_BURST = 400;

/**
 * How much a frame may send in a second, as JSON, and how much at once. Every message
 * is measured by the page before it is read, which takes the page time in proportion.
 */
export const BYTES_PER_SECOND = 4 * 1024 * 1024;
export const BYTES_BURST = 2 * 1024 * 1024;

/** How many messages past its rate a frame may send before it is stopped. */
export const OVER_RATE_MAX = 200;

/** How many of a frame's calls may wait for their answers at once. */
export const CALLS_IN_FLIGHT_MAX = 64;

/** The most a view may have kept for it, as JSON. */
export const STATE_MAX = 64 * 1024;

/** The longest a grant to a stream is. */
const GRANT_MAX = 64;

/** The longest a call's or a state's name may be. */
const NAME_MAX = 128;

/** A message from a frame, once it is known to be one. */
export type FrameMessage =
  | { readonly t: "ready" }
  | { readonly t: "call"; readonly id: number; readonly name: string; readonly args: unknown }
  | { readonly t: "state"; readonly state: unknown }
  | { readonly t: "stream"; readonly id: number; readonly grant: string }
  | {
      readonly t: "key";
      readonly key: string;
      readonly code: string;
      readonly mods: readonly string[];
    }
  | { readonly t: "error"; readonly message: string };

/** Counts what a frame sends against its rate. */
export class Rate {
  #tokens: number;
  #at: number;
  #over = 0;

  constructor(
    private readonly perSecond = RATE_PER_SECOND,
    private readonly burst = RATE_BURST,
    now = 0,
  ) {
    this.#tokens = burst;
    this.#at = now;
  }

  /**
   * Takes one message at `now` (in milliseconds), or `cost` of whatever is counted.
   * Says `ok`, `drop` (past the rate: the message is not taken) or `stop` (past the
   * rate for too long).
   */
  take(now: number, cost = 1): "ok" | "drop" | "stop" {
    const elapsed = Math.max(0, now - this.#at);
    this.#at = now;
    this.#tokens = Math.min(this.burst, this.#tokens + (elapsed / 1000) * this.perSecond);
    if (this.#tokens >= cost) {
      this.#tokens -= cost;
      this.#over = Math.max(0, this.#over - 1);
      return "ok";
    }
    this.#over += 1;
    return this.#over > OVER_RATE_MAX ? "stop" : "drop";
  }
}

/** The size of a value once it is JSON, or none for what JSON cannot hold. */
export function jsonSize(value: unknown): number | null {
  try {
    const text = JSON.stringify(value);
    return text === undefined ? null : text.length;
  } catch {
    return null;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function plainName(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= NAME_MAX;
}

/**
 * Reads what a frame sent. Anything that is not one of its messages, or is larger than
 * a message may be, gives `null`: the frame is then stopped. `measured` is the
 * message's size as JSON, where whoever asks has measured it already.
 */
export function readMessage(data: unknown, measured?: number | null): FrameMessage | null {
  if (!isRecord(data)) return null;
  const size = measured === undefined ? jsonSize(data) : measured;
  if (size === null || size > MESSAGE_MAX) return null;
  switch (data.t) {
    case "ready":
      return { t: "ready" };
    case "call":
      if (typeof data.id !== "number" || !Number.isSafeInteger(data.id) || data.id < 0) return null;
      if (!plainName(data.name)) return null;
      return { t: "call", id: data.id, name: data.name, args: data.args ?? null };
    case "state":
      return { t: "state", state: data.state ?? null };
    case "stream":
      if (typeof data.id !== "number" || !Number.isSafeInteger(data.id) || data.id < 0) return null;
      if (typeof data.grant !== "string" || data.grant.length === 0) return null;
      if (data.grant.length > GRANT_MAX) return null;
      return { t: "stream", id: data.id, grant: data.grant };
    case "key": {
      const mods = Array.isArray(data.mods) ? data.mods : [];
      if (!plainName(data.key) || typeof data.code !== "string" || data.code.length > NAME_MAX)
        return null;
      if (
        !mods.every((mod) => mod === "ctrl" || mod === "alt" || mod === "shift" || mod === "meta")
      )
        return null;
      return { t: "key", key: data.key, code: data.code, mods: mods as string[] };
    }
    case "error":
      return {
        t: "error",
        message: typeof data.message === "string" ? data.message.slice(0, 1000) : "failed",
      };
    default:
      return null;
  }
}

/** What a frame's port is handed on to: the page's side of the bridge. */
export interface FrameHost {
  /** Passes a call on to the plugin's code; settles with its answer. */
  call(name: string, args: unknown): Promise<unknown>;
  /** The frame is set up. */
  ready(): void;
  /** Keeps what the frame wants to find again. */
  state(state: unknown): void;
  /** A key the frame did not use. */
  key(key: { key: string; code: string; mods: readonly string[] }): void;
  /**
   * Opens a stream the frame was handed a grant to; settles with the frame's end of
   * it, and whether it is read, written or both.
   */
  stream(grant: string): Promise<{
    readonly port: Transferable;
    readonly readable: boolean;
    readonly writable: boolean;
  }>;
  /** The frame must be stopped: it failed, or broke the rules. */
  stop(reason: string): void;
}

/** The part of a port the bridge uses, so tests can stand one in. */
export interface PortLike {
  postMessage(message: unknown, transfer?: Transferable[]): void;
  onmessage: ((event: MessageEvent) => void) | null;
  close(): void;
}

/** An error as a frame is told it: its kind and what happened. */
function failure(error: unknown): { code: string; message: string; data?: unknown } {
  if (isRecord(error)) {
    const code = typeof error.code === "string" ? error.code : "failed";
    const message = typeof error.message === "string" ? error.message : String(error);
    // What the plugin's code said of a failure of its own goes to its view as it is.
    const data = error.data;
    return data === undefined || data === null ? { code, message } : { code, message, data };
  }
  return { code: "failed", message: error instanceof Error ? error.message : String(error) };
}

/** One frame's end of the bridge, on the page's side. */
export class FrameBridge {
  readonly #rate: Rate;
  readonly #bytes: Rate;
  #inFlight = 0;
  #stopped = false;

  constructor(
    private readonly port: PortLike,
    private readonly host: FrameHost,
    private readonly now: () => number = () => performance.now(),
  ) {
    this.#rate = new Rate(RATE_PER_SECOND, RATE_BURST, now());
    this.#bytes = new Rate(BYTES_PER_SECOND, BYTES_BURST, now());
    port.onmessage = (event) => this.#receive(event.data);
  }

  get stopped(): boolean {
    return this.#stopped;
  }

  #stop(reason: string): void {
    if (this.#stopped) return;
    this.#stopped = true;
    this.port.onmessage = null;
    this.port.close();
    this.host.stop(reason);
  }

  #receive(data: unknown): void {
    if (this.#stopped) return;
    switch (this.#rate.take(this.now())) {
      case "drop":
        return;
      case "stop":
        this.#stop("it sent far more messages than a view may");
        return;
      case "ok":
        break;
    }
    // Measuring a message takes the page time in proportion to its size, so what a
    // view may send in a second is limited as well as how often it may send.
    const size = jsonSize(data);
    const message = readMessage(data, size);
    if (message === null) {
      this.#stop("it sent something that is not a message a view may send");
      return;
    }
    if (this.#bytes.take(this.now(), size ?? 0) !== "ok") {
      this.#stop("it sent far more than a view may");
      return;
    }
    switch (message.t) {
      case "ready":
        this.host.ready();
        return;
      case "call":
        this.#call(message.id, message.name, message.args);
        return;
      case "state": {
        const size = jsonSize(message.state);
        if (size === null || size > STATE_MAX) {
          this.#post({
            t: "refused",
            what: "state",
            message: `a view keeps ${STATE_MAX / 1024} KiB at most`,
          });
          return;
        }
        this.host.state(message.state);
        return;
      }
      case "stream":
        this.#stream(message.id, message.grant);
        return;
      case "key":
        this.host.key(message);
        return;
      case "error":
        this.#stop(`it failed: ${message.message}`);
        return;
    }
  }

  #call(id: number, name: string, args: unknown): void {
    if (this.#inFlight >= CALLS_IN_FLIGHT_MAX) {
      this.#post({
        t: "error",
        id,
        code: "busy",
        message: `a view waits for ${CALLS_IN_FLIGHT_MAX} answers at most`,
      });
      return;
    }
    this.#inFlight += 1;
    this.host.call(name, args).then(
      (value) => {
        this.#inFlight -= 1;
        this.#post({ t: "result", id, value: value ?? null });
      },
      (error: unknown) => {
        this.#inFlight -= 1;
        this.#post({ t: "error", id, ...failure(error) });
      },
    );
  }

  #stream(id: number, grant: string): void {
    if (this.#inFlight >= CALLS_IN_FLIGHT_MAX) {
      this.#post({ t: "error", id, code: "busy", message: "the view asks for too much at once" });
      return;
    }
    this.#inFlight += 1;
    this.host.stream(grant).then(
      (opened) => {
        this.#inFlight -= 1;
        this.#post(
          {
            t: "opened",
            id,
            port: opened.port,
            readable: opened.readable,
            writable: opened.writable,
          },
          [opened.port],
        );
      },
      (error: unknown) => {
        this.#inFlight -= 1;
        this.#post({ t: "error", id, ...failure(error) });
      },
    );
  }

  #post(message: unknown, transfer?: Transferable[]): void {
    if (this.#stopped) return;
    try {
      this.port.postMessage(message, transfer);
    } catch {
      this.#stop("it could not be sent what it was owed");
    }
  }

  /** Passes on an event from the plugin's code. */
  event(event: string, payload: unknown): void {
    this.#post({ t: "event", event, payload });
  }

  /** Tells the frame the window's look. */
  tokens(tokens: Readonly<Record<string, string>>, scheme: string): void {
    this.#post({ t: "tokens", tokens, scheme });
  }

  /** Tells the frame it has the keyboard. */
  focus(): void {
    this.#post({ t: "focus" });
  }

  /** Ends the bridge from the page's side. */
  close(): void {
    if (this.#stopped) return;
    this.#stopped = true;
    this.port.onmessage = null;
    this.port.close();
  }
}

/** What the page first hands a frame, with the port. */
export interface Hello {
  readonly lume: "connect";
  readonly api: 1;
  readonly instance: string;
  readonly plugin: string;
  readonly view: string;
  readonly place: "pane" | "panel" | "settings";
  readonly state: unknown;
  readonly tokens: Readonly<Record<string, string>>;
  readonly scheme: string;
}

/** Where plugins' pages are served from, on this system, without a `/` at its end. */
export function pluginBase(userAgent: string = globalThis.navigator?.userAgent ?? ""): string {
  return /Windows/i.test(userAgent) ? "http://lume-plugin.localhost" : "lume-plugin://localhost";
}

/** The address of a plugin's page. Each part of the path is encoded on its own. */
export function pageUrl(base: string, plugin: string, entry: string): string {
  const path = entry.split("/").map(encodeURIComponent).join("/");
  return `${base}/${encodeURIComponent(plugin)}/${path}`;
}
