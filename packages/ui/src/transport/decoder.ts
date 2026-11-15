/**
 * Turns the bytes the core sends back into frames, whatever sizes the bytes arrive in.
 *
 * The rules are the ones the core's own decoder applies: a frame of a kind only the page
 * sends, a payload over the limit, a control message off the control stream, or a split
 * message of a kind that may not be split, each ends the stream of frames for good. After
 * a framing error nobody knows where the next frame starts.
 */

import { CONTROL_STREAM, HEADER_LEN, type HostFrame, Kind, KIND_MORE } from "./frames.ts";
import { type ExitStatus, isTermSize, type SessionEvent, type TermSize } from "./types.ts";

/** The sizes a decoder accepts. The core reports them when the carrier opens. */
export interface FrameLimits {
  /** The most bytes one frame's payload may have. */
  readonly maxPayload: number;
  /** The most bytes a split control message may have once joined. */
  readonly maxControlMessage: number;
  /** The most bytes a split hydrate message may have once joined. */
  readonly maxHydrate: number;
}

export type FrameErrorCode =
  | "unknown_kind"
  | "wrong_direction"
  | "payload_too_large"
  | "wrong_plane"
  | "not_fragmentable"
  | "empty_fragment"
  | "interrupted"
  | "message_too_large"
  | "bad_length"
  | "bad_payload";

/** Why the bytes are not a valid stream of frames. */
export class FrameError extends Error {
  readonly code: FrameErrorCode;

  constructor(code: FrameErrorCode, message: string) {
    super(message);
    this.name = "FrameError";
    this.code = code;
  }
}

const RESIZED_LEN = 4;
const EXIT_LEN = 5;

const EXIT_CODE = 0;
const EXIT_SIGNAL = 1;
const EXIT_UNKNOWN = 2;

const HYDRATE_SNAPSHOT = 1;
const HYDRATE_REPLAY = 2;

const HOST_KINDS: ReadonlySet<number> = new Set([
  Kind.Control,
  Kind.Hydrate,
  Kind.Data,
  Kind.Resized,
  Kind.Event,
  Kind.Exit,
]);
const CLIENT_KINDS: ReadonlySet<number> = new Set([
  Kind.Input,
  Kind.Resize,
  Kind.Facts,
  Kind.Credit,
  Kind.Detach,
]);

const EMPTY = new Uint8Array(0);

interface Unfinished {
  readonly kind: number;
  readonly parts: Uint8Array[];
  length: number;
}

/**
 * An incremental decoder for what the core sends.
 *
 * Give it bytes with {@link push} as they arrive and take frames with {@link next} until
 * it returns `null`. Output bytes are handed over as views of the pushed chunk, not
 * copies, so a caller that keeps them past the next push must copy them itself.
 */
export class FrameDecoder {
  readonly #limits: FrameLimits;
  #buffer: Uint8Array = EMPTY;
  #read = 0;
  readonly #unfinished = new Map<number, Unfinished>();
  #failed: FrameError | undefined;
  readonly #text = new TextDecoder("utf-8", { fatal: true });

  constructor(limits: FrameLimits) {
    this.#limits = limits;
  }

  /** Adds received bytes. The decoder keeps `chunk`; the caller must not change it. */
  push(chunk: Uint8Array): void {
    if (this.#failed) return;
    const left = this.#buffer.length - this.#read;
    if (left === 0) {
      // The usual case: the last chunk ended on a frame boundary.
      this.#buffer = chunk;
    } else {
      const joined = new Uint8Array(left + chunk.length);
      joined.set(this.#buffer.subarray(this.#read));
      joined.set(chunk, left);
      this.#buffer = joined;
    }
    this.#read = 0;
  }

  /** The next complete frame, or `null` when more bytes are needed. */
  next(): HostFrame | null {
    if (this.#failed) throw this.#failed;
    try {
      return this.#advance();
    } catch (error) {
      this.#failed =
        error instanceof FrameError ? error : new FrameError("bad_payload", String(error));
      this.#buffer = EMPTY;
      this.#read = 0;
      this.#unfinished.clear();
      throw this.#failed;
    }
  }

  #advance(): HostFrame | null {
    for (;;) {
      const available = this.#buffer.length - this.#read;
      if (available < HEADER_LEN) return null;
      const view = new DataView(this.#buffer.buffer, this.#buffer.byteOffset + this.#read);
      const stream = view.getUint32(0, true);
      const kindByte = view.getUint8(4);
      const len = view.getUint32(5, true);
      const more = (kindByte & KIND_MORE) !== 0;
      const kind = kindByte & ~KIND_MORE;

      // The header is judged before its payload arrives, so a bad frame is refused
      // without waiting for bytes that may never come.
      if (CLIENT_KINDS.has(kind)) {
        throw new FrameError("wrong_direction", `the core sent a frame of kind ${hex(kind)}`);
      }
      if (!HOST_KINDS.has(kind)) {
        throw new FrameError("unknown_kind", `unknown frame kind ${hex(kindByte)}`);
      }
      if (len > this.#limits.maxPayload) {
        throw new FrameError("payload_too_large", `a payload of ${len} bytes`);
      }
      if ((kind === Kind.Control) !== (stream === CONTROL_STREAM)) {
        throw new FrameError("wrong_plane", `kind ${hex(kind)} on stream ${stream}`);
      }
      const fragmentable = kind === Kind.Control || kind === Kind.Hydrate;
      if (more && !fragmentable) {
        throw new FrameError("not_fragmentable", `kind ${hex(kind)} may not be split`);
      }
      if (more && len === 0) {
        throw new FrameError("empty_fragment", "a continued frame with no payload");
      }

      const end = HEADER_LEN + len;
      if (available < end) return null;
      const payload = this.#buffer.subarray(this.#read + HEADER_LEN, this.#read + end);

      let complete: Uint8Array | undefined;
      const unfinished = this.#unfinished.get(stream);
      if (unfinished) {
        if (unfinished.kind !== kind) {
          throw new FrameError(
            "interrupted",
            `stream ${stream}: kind ${hex(kind)} inside a message of kind ${hex(unfinished.kind)}`,
          );
        }
        if (unfinished.length + payload.length > this.#messageLimit(kind)) {
          throw new FrameError("message_too_large", `a message of kind ${hex(kind)}`);
        }
        unfinished.parts.push(payload);
        unfinished.length += payload.length;
        if (!more) {
          this.#unfinished.delete(stream);
          complete = join(unfinished);
        }
      } else if (more) {
        this.#unfinished.set(stream, { kind, parts: [payload], length: payload.length });
      } else {
        complete = payload;
      }
      this.#read += end;

      if (complete) return this.#decode(stream, kind, complete);
    }
  }

  #messageLimit(kind: number): number {
    return kind === Kind.Hydrate ? this.#limits.maxHydrate : this.#limits.maxControlMessage;
  }

  #decode(stream: number, kind: number, bytes: Uint8Array): HostFrame {
    switch (kind) {
      case Kind.Control:
        return { stream, kind: "control", message: this.#json(kind, bytes) };
      case Kind.Hydrate:
        return this.#hydrate(stream, bytes);
      case Kind.Data:
        return { stream, kind: "data", bytes };
      case Kind.Resized: {
        const view = fixed(kind, bytes, RESIZED_LEN);
        return {
          stream,
          kind: "resized",
          size: size(kind, view.getUint16(0, true), view.getUint16(2, true)),
        };
      }
      case Kind.Event:
        return { stream, kind: "event", event: event(this.#json(kind, bytes)) };
      default:
        return { stream, kind: "exit", status: exit(bytes) };
    }
  }

  #json(kind: number, bytes: Uint8Array): unknown {
    try {
      return JSON.parse(this.#text.decode(bytes));
    } catch (error) {
      throw new FrameError("bad_payload", `kind ${hex(kind)}: ${String(error)}`);
    }
  }

  #hydrate(stream: number, bytes: Uint8Array): HostFrame {
    const form = bytes[0];
    if (form === HYDRATE_REPLAY) {
      return { stream, kind: "hydrate", form: "vt_replay", bytes: bytes.subarray(1) };
    }
    if (form === HYDRATE_SNAPSHOT) {
      const versionLen = bytes[1];
      if (versionLen === undefined) throw bad(Kind.Hydrate, "a snapshot has no engine version");
      if (bytes.length < 2 + versionLen) throw bad(Kind.Hydrate, "the engine version is cut short");
      let vtVersion: string;
      try {
        vtVersion = this.#text.decode(bytes.subarray(2, 2 + versionLen));
      } catch {
        throw bad(Kind.Hydrate, "the engine version is not UTF-8");
      }
      return {
        stream,
        kind: "hydrate",
        form: "snapshot",
        vtVersion,
        bytes: bytes.subarray(2 + versionLen),
      };
    }
    throw bad(Kind.Hydrate, form === undefined ? "empty" : "unknown form");
  }
}

function hex(byte: number): string {
  return `0x${byte.toString(16).padStart(2, "0")}`;
}

function bad(kind: number, reason: string): FrameError {
  return new FrameError("bad_payload", `kind ${hex(kind)}: ${reason}`);
}

function join(message: Unfinished): Uint8Array {
  const out = new Uint8Array(message.length);
  let at = 0;
  for (const part of message.parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

function fixed(kind: number, bytes: Uint8Array, expected: number): DataView {
  if (bytes.length !== expected) {
    throw new FrameError(
      "bad_length",
      `kind ${hex(kind)}: ${bytes.length} bytes where ${expected} are expected`,
    );
  }
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
}

function size(kind: number, cols: number, rows: number): TermSize {
  const value = { cols, rows };
  if (!isTermSize(value)) throw bad(kind, `a terminal of ${cols} by ${rows} cells`);
  return value;
}

function exit(bytes: Uint8Array): ExitStatus {
  const view = fixed(Kind.Exit, bytes, EXIT_LEN);
  const how = view.getUint8(0);
  const value = view.getUint32(1, true);
  if (how === EXIT_CODE) return { code: value };
  if (how === EXIT_SIGNAL) return { signal: value };
  if (how !== EXIT_UNKNOWN) throw bad(Kind.Exit, "unknown status form");
  if (value !== 0) throw bad(Kind.Exit, "an unknown status carries a value");
  return "unknown";
}

function event(value: unknown): SessionEvent {
  if (typeof value === "object" && value !== null) {
    const { type, title, url } = value as { type?: unknown; title?: unknown; url?: unknown };
    if (type === "title" && typeof title === "string") return { type, title };
    if (type === "cwd" && typeof url === "string") return { type, url };
    if (type === "bell") return { type };
  }
  throw bad(Kind.Event, "not a session event");
}
