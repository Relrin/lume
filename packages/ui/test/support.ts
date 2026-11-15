/**
 * Shared by the transport tests: the recorded byte streams the core's own tests check its
 * codec against, and the limits a decoder is given.
 */

import { readFileSync } from "node:fs";

import type { FrameLimits } from "../src/transport/decoder.ts";

/** The limits the recorded streams were written under. */
export const LIMITS: FrameLimits = {
  maxPayload: 64 * 1024,
  maxControlMessage: 1024 * 1024,
  maxHydrate: 64 * 1024 * 1024,
};

const VECTORS = "../../../crates/lume-proto/tests/vectors/";

/** One recorded stream: the bytes on the wire and a description of each message in them. */
export function vector(name: string): { bytes: Uint8Array; messages: Record<string, unknown>[] } {
  const bytes = new Uint8Array(readFileSync(new URL(`${VECTORS}${name}.bin`, import.meta.url)));
  const json = readFileSync(new URL(`${VECTORS}${name}.json`, import.meta.url), "utf8");
  return { bytes, messages: JSON.parse(json) as Record<string, unknown>[] };
}

export function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function fromHex(text: string): Uint8Array {
  const out = new Uint8Array(text.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(text.slice(2 * i, 2 * i + 2), 16);
  return out;
}

/** A frame with any header and payload, valid or not. */
export function rawFrame(
  stream: number,
  kindByte: number,
  payload: Uint8Array | number[],
): Uint8Array {
  const body = payload instanceof Uint8Array ? payload : Uint8Array.from(payload);
  const out = new Uint8Array(9 + body.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, stream, true);
  view.setUint8(4, kindByte);
  view.setUint32(5, body.length, true);
  out.set(body, 9);
  return out;
}

/** A header that announces `len` payload bytes, with none of them following. */
export function rawHeader(stream: number, kindByte: number, len: number): Uint8Array {
  const out = new Uint8Array(9);
  const view = new DataView(out.buffer);
  view.setUint32(0, stream, true);
  view.setUint8(4, kindByte);
  view.setUint32(5, len, true);
  return out;
}

export function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}
