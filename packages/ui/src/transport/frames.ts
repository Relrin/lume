/**
 * The frames that travel between this page and the core, and how the page writes its own.
 *
 * A frame is a nine-byte header, `[u32 stream][u8 kind][u32 len]` in little-endian order,
 * followed by `len` payload bytes. The top bit of the kind byte says the message continues
 * in the next frame of the same stream and kind. This is the framing the core uses towards
 * its session host, so the two ends are checked against the same recorded byte streams.
 */

import type { CellPx, ClientFacts, ExitStatus, SessionEvent, TermSize } from "./types.ts";

/** What a frame carries. The values are the wire encoding. */
export const Kind = {
  Control: 0x01,
  Hydrate: 0x02,
  Data: 0x03,
  Resized: 0x04,
  Event: 0x05,
  Exit: 0x06,
  Input: 0x10,
  Resize: 0x11,
  Facts: 0x12,
  Credit: 0x13,
  Detach: 0x14,
  Rehydrate: 0x15,
} as const;

/** The encoded size of a frame header. */
export const HEADER_LEN = 9;

/** The bit of the kind byte that says the message continues in the next frame. */
export const KIND_MORE = 0x80;

/** The stream that carries control messages. Every other stream is one attachment. */
export const CONTROL_STREAM = 0;

const RESIZE_LEN = 8;
const FACTS_LEN = 67;
const CREDIT_LEN = 4;

const FACTS_FOCUSED = 1 << 0;
const FACTS_VISIBLE = 1 << 1;
const FACTS_DARK = 1 << 2;

/** One decoded message from the core and the stream it arrived on. */
export type HostFrame =
  | { readonly stream: number; readonly kind: "control"; readonly message: unknown }
  | {
      readonly stream: number;
      readonly kind: "hydrate";
      readonly form: "snapshot";
      readonly vtVersion: string;
      readonly bytes: Uint8Array;
    }
  | {
      readonly stream: number;
      readonly kind: "hydrate";
      readonly form: "vt_replay";
      readonly bytes: Uint8Array;
    }
  | { readonly stream: number; readonly kind: "data"; readonly bytes: Uint8Array }
  | { readonly stream: number; readonly kind: "resized"; readonly size: TermSize }
  | { readonly stream: number; readonly kind: "event"; readonly event: SessionEvent }
  | { readonly stream: number; readonly kind: "exit"; readonly status: ExitStatus };

function header(out: Uint8Array, at: number, stream: number, kind: number, len: number): void {
  const view = new DataView(out.buffer, out.byteOffset + at, HEADER_LEN);
  view.setUint32(0, stream, true);
  view.setUint8(4, kind);
  view.setUint32(5, len, true);
}

function single(stream: number, kind: number, len: number): { out: Uint8Array; body: DataView } {
  const out = new Uint8Array(HEADER_LEN + len);
  header(out, 0, stream, kind, len);
  return { out, body: new DataView(out.buffer, HEADER_LEN, len) };
}

/**
 * Bytes for the session's program, as if typed. A run longer than one payload becomes
 * several frames; terminal bytes are a stream, so the receiver simply joins them.
 */
export function encodeInput(stream: number, bytes: Uint8Array, maxPayload: number): Uint8Array {
  const frames = Math.max(1, Math.ceil(bytes.length / maxPayload));
  const out = new Uint8Array(bytes.length + frames * HEADER_LEN);
  let at = 0;
  for (let from = 0, n = 0; n < frames; n++) {
    const len = Math.min(maxPayload, bytes.length - from);
    header(out, at, stream, Kind.Input, len);
    out.set(bytes.subarray(from, from + len), at + HEADER_LEN);
    at += HEADER_LEN + len;
    from += len;
  }
  return out;
}

/** The size the view wants its terminal to have. */
export function encodeResize(stream: number, size: TermSize, cellPx: CellPx): Uint8Array {
  const { out, body } = single(stream, Kind.Resize, RESIZE_LEN);
  body.setUint16(0, size.cols, true);
  body.setUint16(2, size.rows, true);
  body.setUint16(4, cellPx.w, true);
  body.setUint16(6, cellPx.h, true);
  return out;
}

/** How the view has changed. */
export function encodeFacts(stream: number, facts: ClientFacts): Uint8Array {
  const { out, body } = single(stream, Kind.Facts, FACTS_LEN);
  let flags = 0;
  if (facts.focused) flags |= FACTS_FOCUSED;
  if (facts.visible) flags |= FACTS_VISIBLE;
  if (facts.scheme === "dark") flags |= FACTS_DARK;
  body.setUint8(0, flags);
  body.setUint16(2, facts.size.cols, true);
  body.setUint16(4, facts.size.rows, true);
  body.setUint16(6, facts.cellPx.w, true);
  body.setUint16(8, facts.cellPx.h, true);
  const { palette } = facts;
  const colours = [palette.fg, palette.bg, palette.cursor, ...palette.ansi];
  if (colours.length !== 19) throw new RangeError("a palette has sixteen ANSI colours");
  let at = 10;
  for (const colour of colours) {
    body.setUint8(at++, colour.r);
    body.setUint8(at++, colour.g);
    body.setUint8(at++, colour.b);
  }
  return out;
}

/** Lets the session send `bytes` more of output. */
export function encodeCredit(stream: number, bytes: number): Uint8Array {
  const { out, body } = single(stream, Kind.Credit, CREDIT_LEN);
  body.setUint32(0, bytes, true);
  return out;
}

/** Ends the attachment. The session carries on. */
export function encodeDetach(stream: number): Uint8Array {
  return single(stream, Kind.Detach, 0).out;
}

/** Asks for the state to start again from, as of this point of the output. */
export function encodeRehydrate(stream: number): Uint8Array {
  return single(stream, Kind.Rehydrate, 0).out;
}
