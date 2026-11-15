import { describe, expect, it } from "vitest";

import { FrameDecoder, FrameError } from "../src/transport/decoder.ts";
import type { HostFrame } from "../src/transport/frames.ts";
import { concat, LIMITS, rawFrame, rawHeader, toHex, vector } from "./support.ts";

/** A frame in the words the recorded description uses. */
function describeFrame(frame: HostFrame): Record<string, unknown> {
  switch (frame.kind) {
    case "control":
      return { kind: "control", message: frame.message, stream: frame.stream };
    case "hydrate":
      return frame.form === "snapshot"
        ? {
            bytes: toHex(frame.bytes),
            form: "snapshot",
            kind: "hydrate",
            stream: frame.stream,
            vt_version: frame.vtVersion,
          }
        : { bytes: toHex(frame.bytes), form: "vt_replay", kind: "hydrate", stream: frame.stream };
    case "data":
      return { bytes: toHex(frame.bytes), kind: "data", stream: frame.stream };
    case "resized":
      return { kind: "resized", size: frame.size, stream: frame.stream };
    case "event":
      return { event: frame.event, kind: "event", stream: frame.stream };
    case "exit":
      return { kind: "exit", status: frame.status, stream: frame.stream };
  }
}

function drain(decoder: FrameDecoder, into: Record<string, unknown>[]): void {
  for (let frame = decoder.next(); frame; frame = decoder.next()) into.push(describeFrame(frame));
}

function decodeAll(...chunks: Uint8Array[]): Record<string, unknown>[] {
  const decoder = new FrameDecoder(LIMITS);
  const out: Record<string, unknown>[] = [];
  for (const chunk of chunks) {
    decoder.push(chunk);
    drain(decoder, out);
  }
  return out;
}

function failure(...chunks: Uint8Array[]): string {
  try {
    decodeAll(...chunks);
  } catch (error) {
    if (error instanceof FrameError) return error.code;
    throw error;
  }
  return "no error";
}

const DATA = 0x03;
const RESIZED = 0x04;
const EVENT = 0x05;
const EXIT = 0x06;
const HYDRATE = 0x02;
const CONTROL = 0x01;
const MORE = 0x80;

describe("the frame decoder", () => {
  const recorded = vector("host-to-client");

  it("reads the recorded stream the core's own decoder is checked against", () => {
    expect(decodeAll(recorded.bytes)).toEqual(recorded.messages);
  });

  it("reads the same messages wherever the stream is cut in two", () => {
    for (let cut = 1; cut < recorded.bytes.length; cut++) {
      // Copies, because a decoder keeps the chunks it is given.
      const head = recorded.bytes.slice(0, cut);
      const tail = recorded.bytes.slice(cut);
      expect(decodeAll(head, tail), `cut at ${cut}`).toEqual(recorded.messages);
    }
  });

  it("reads the same messages when the stream arrives a byte at a time", () => {
    const bytes = Array.from(recorded.bytes, (byte) => Uint8Array.of(byte));
    expect(decodeAll(...bytes)).toEqual(recorded.messages);
  });

  it("hands output over as a view of the chunk it arrived in", () => {
    const chunk = rawFrame(1, DATA, [1, 2, 3]);
    const decoder = new FrameDecoder(LIMITS);
    decoder.push(chunk);
    const frame = decoder.next();
    expect(frame?.kind === "data" && frame.bytes.buffer === chunk.buffer).toBe(true);
  });

  it("accepts a payload of exactly the limit and refuses one byte more", () => {
    const full = decodeAll(rawFrame(1, DATA, new Uint8Array(LIMITS.maxPayload)));
    expect(full).toHaveLength(1);
    expect(failure(rawHeader(1, DATA, LIMITS.maxPayload + 1))).toBe("payload_too_large");
  });

  it("refuses a bad header before its payload has arrived", () => {
    expect(failure(rawHeader(1, 0x7f, 100))).toBe("unknown_kind");
  });

  it("refuses the kinds only the page sends", () => {
    for (const kind of [0x10, 0x11, 0x12, 0x13, 0x14]) {
      expect(failure(rawFrame(1, kind, [])), `kind ${kind}`).toBe("wrong_direction");
    }
  });

  it("refuses a control message off the control stream and anything else on it", () => {
    expect(failure(rawFrame(1, CONTROL, [0x7b, 0x7d]))).toBe("wrong_plane");
    expect(failure(rawFrame(0, DATA, [1]))).toBe("wrong_plane");
  });

  it("refuses to split a kind that is not a control or hydrate message", () => {
    expect(failure(rawFrame(1, DATA | MORE, [1]))).toBe("not_fragmentable");
  });

  it("refuses a continued frame with no payload", () => {
    expect(failure(rawFrame(1, HYDRATE | MORE, []))).toBe("empty_fragment");
  });

  it("joins a split message and lets other streams come in between", () => {
    const frames = concat(
      rawFrame(1, HYDRATE | MORE, [2, 0x61]),
      rawFrame(2, DATA, [9]),
      rawFrame(1, HYDRATE, [0x62]),
    );
    expect(decodeAll(frames)).toEqual([
      { bytes: "09", kind: "data", stream: 2 },
      { bytes: "6162", form: "vt_replay", kind: "hydrate", stream: 1 },
    ]);
  });

  it("refuses another kind in the middle of a split message", () => {
    const frames = concat(rawFrame(1, HYDRATE | MORE, [2, 0x61]), rawFrame(1, DATA, [9]));
    expect(failure(frames)).toBe("interrupted");
  });

  it("refuses a split message that grows past its limit", () => {
    const decoder = new FrameDecoder({ ...LIMITS, maxHydrate: 5 });
    decoder.push(concat(rawFrame(1, HYDRATE | MORE, [2, 1, 2]), rawFrame(1, HYDRATE, [3, 4, 5])));
    expect(() => decoder.next()).toThrow(FrameError);
  });

  it("refuses fixed-size payloads of the wrong length", () => {
    expect(failure(rawFrame(1, RESIZED, [80, 0, 24]))).toBe("bad_length");
    expect(failure(rawFrame(1, EXIT, [0, 0, 0, 0]))).toBe("bad_length");
  });

  it("refuses a size no terminal may have", () => {
    expect(failure(rawFrame(1, RESIZED, [0, 0, 24, 0]))).toBe("bad_payload");
    expect(failure(rawFrame(1, RESIZED, [0x01, 0x10, 24, 0]))).toBe("bad_payload");
  });

  it("refuses an exit status it does not know, or an unknown one with a value", () => {
    expect(failure(rawFrame(1, EXIT, [3, 0, 0, 0, 0]))).toBe("bad_payload");
    expect(failure(rawFrame(1, EXIT, [2, 1, 0, 0, 0]))).toBe("bad_payload");
  });

  it("refuses a hydrate message that is empty, of an unknown form, or cut short", () => {
    expect(failure(rawFrame(1, HYDRATE, []))).toBe("bad_payload");
    expect(failure(rawFrame(1, HYDRATE, [9, 1]))).toBe("bad_payload");
    expect(failure(rawFrame(1, HYDRATE, [1]))).toBe("bad_payload");
    expect(failure(rawFrame(1, HYDRATE, [1, 5, 0x31]))).toBe("bad_payload");
    expect(failure(rawFrame(1, HYDRATE, [1, 1, 0xff]))).toBe("bad_payload");
  });

  it("refuses an event that is not JSON or not an event it knows", () => {
    const text = (value: string) => new TextEncoder().encode(value);
    expect(failure(rawFrame(1, EVENT, text("{")))).toBe("bad_payload");
    expect(failure(rawFrame(1, EVENT, [0xff, 0xfe]))).toBe("bad_payload");
    expect(failure(rawFrame(1, EVENT, text('{"type":"mystery"}')))).toBe("bad_payload");
    expect(failure(rawFrame(1, EVENT, text('{"type":"title"}')))).toBe("bad_payload");
  });

  it("stays failed once it has failed", () => {
    const decoder = new FrameDecoder(LIMITS);
    decoder.push(rawFrame(1, 0x7f, []));
    expect(() => decoder.next()).toThrow(FrameError);
    decoder.push(rawFrame(1, DATA, [1]));
    expect(() => decoder.next()).toThrow(FrameError);
  });
});
