import { describe, expect, it } from "vitest";

import {
  encodeCredit,
  encodeDetach,
  encodeFacts,
  encodeInput,
  encodeResize,
  HEADER_LEN,
} from "../src/transport/frames.ts";
import type { ClientFacts, Rgb, TermSize } from "../src/transport/types.ts";
import { concat, fromHex, LIMITS, toHex, vector } from "./support.ts";

function rgb(text: string): Rgb {
  const n = parseInt(text.slice(1), 16);
  return { r: n >> 16, g: (n >> 8) & 0xff, b: n & 0xff };
}

interface WireFacts {
  cell_px: { w: number; h: number };
  focused: boolean;
  palette: { ansi: string[]; bg: string; cursor: string; fg: string };
  scheme: "light" | "dark";
  size: TermSize;
  visible: boolean;
}

function facts(wire: WireFacts): ClientFacts {
  return {
    size: wire.size,
    cellPx: wire.cell_px,
    palette: {
      fg: rgb(wire.palette.fg),
      bg: rgb(wire.palette.bg),
      cursor: rgb(wire.palette.cursor),
      ansi: wire.palette.ansi.map(rgb),
    },
    scheme: wire.scheme,
    focused: wire.focused,
    visible: wire.visible,
  };
}

/** Cuts a recorded stream into its frames. */
function framesOf(bytes: Uint8Array): Uint8Array[] {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
  const out: Uint8Array[] = [];
  for (let at = 0; at < bytes.length;) {
    const end = at + HEADER_LEN + view.getUint32(at + 5, true);
    out.push(bytes.subarray(at, end));
    at = end;
  }
  return out;
}

describe("the frames the page writes", () => {
  it("are byte for byte the ones in the stream the core's decoder is checked against", () => {
    const recorded = vector("client-to-host");
    const frames = framesOf(recorded.bytes);
    // Every message here fits one frame, so the two lists line up.
    expect(frames).toHaveLength(recorded.messages.length);

    let compared = 0;
    recorded.messages.forEach((message, index) => {
      const stream = message.stream as number;
      let encoded: Uint8Array;
      switch (message.kind) {
        case "credit":
          encoded = encodeCredit(stream, message.bytes as number);
          break;
        case "resize":
          encoded = encodeResize(
            stream,
            message.size as TermSize,
            message.cell_px as { w: number; h: number },
          );
          break;
        case "facts":
          encoded = encodeFacts(stream, facts(message.facts as WireFacts));
          break;
        case "input":
          encoded = encodeInput(stream, fromHex(message.bytes as string), LIMITS.maxPayload);
          break;
        case "detach":
          encoded = encodeDetach(stream);
          break;
        default:
          // Control messages do not travel this way from the page.
          return;
      }
      expect(toHex(encoded), `message ${index} (${String(message.kind)})`).toBe(
        toHex(frames[index]!),
      );
      compared++;
    });
    expect(compared).toBe(6);
  });

  it("split input longer than one payload into several frames that join to the original", () => {
    const bytes = Uint8Array.from({ length: 25 }, (_, i) => i);
    const frames = framesOf(encodeInput(3, bytes, 10));
    expect(frames.map((frame) => frame.length - HEADER_LEN)).toEqual([10, 10, 5]);
    expect(concat(...frames.map((frame) => frame.subarray(HEADER_LEN)))).toEqual(bytes);
    for (const frame of frames) expect(frame[4]).toBe(0x10);
  });

  it("refuse a palette that does not have sixteen ANSI colours", () => {
    const black = { r: 0, g: 0, b: 0 };
    const short: ClientFacts = {
      size: { cols: 80, rows: 24 },
      cellPx: { w: 8, h: 16 },
      palette: { fg: black, bg: black, cursor: black, ansi: [black] },
      scheme: "dark",
      focused: true,
      visible: true,
    };
    expect(() => encodeFacts(1, short)).toThrow(RangeError);
  });
});
