import { beforeAll, describe, expect, it } from "vitest";

import {
  encodeFocus,
  encodePaste,
  isPasteSafe,
  Mod,
  MouseEncoder,
  type MouseInput,
  type Terminal,
  type Vt,
} from "../src/index.ts";
import { loadVt } from "./support.ts";

let vt: Vt;
beforeAll(async () => {
  vt = await loadVt();
});

const ESC = "\x1b";
const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

function withTerminal(setup: string, body: (term: Terminal) => void): void {
  const term = vt.createTerminal(80, 24);
  try {
    term.write(setup);
    body(term);
  } finally {
    term.free();
  }
}

describe("focus reports", () => {
  it("encodes gaining and losing focus", () => {
    expect(text(encodeFocus(vt, true))).toBe(`${ESC}[I`);
    expect(text(encodeFocus(vt, false))).toBe(`${ESC}[O`);
  });
});

describe("paste", () => {
  it("accepts plain single-line text as safe", () => {
    expect(isPasteSafe(vt, "ls -la")).toBe(true);
    expect(isPasteSafe(vt, "")).toBe(true);
    expect(isPasteSafe(vt, "日本語 ✓")).toBe(true);
  });

  it("flags a line break or the end-of-paste marker as unsafe", () => {
    expect(isPasteSafe(vt, "rm -rf ~\n")).toBe(false);
    expect(isPasteSafe(vt, "a\rb")).toBe(false);
    expect(isPasteSafe(vt, `x${ESC}[201~y`)).toBe(false);
  });

  it("types line breaks as carriage returns when the program has no bracketed paste", () => {
    withTerminal("", (term) => {
      expect(text(encodePaste(term, "one\ntwo"))).toBe("one\rtwo");
    });
  });

  it("wraps the text in markers when the program has bracketed paste on", () => {
    withTerminal(`${ESC}[?2004h`, (term) => {
      expect(text(encodePaste(term, "one\ntwo"))).toBe(`${ESC}[200~one\ntwo${ESC}[201~`);
    });
  });

  it("replaces control characters that could act on the terminal", () => {
    withTerminal(`${ESC}[?2004h`, (term) => {
      const pasted = text(encodePaste(term, `a${ESC}[201~b\x00c`));
      expect(pasted).toBe(`${ESC}[200~a [201~b c${ESC}[201~`);
    });
  });

  it("encodes a paste larger than the first buffer it is given", () => {
    withTerminal("", (term) => {
      const big = "x".repeat(100_000);
      expect(text(encodePaste(term, big))).toBe(big);
      // The buffer has grown; a small paste afterwards is unaffected.
      expect(text(encodePaste(term, "y"))).toBe("y");
    });
  });
});

describe("mouse", () => {
  // Cells of 10 by 20 pixels, so the pixel (25, 70) is column 3, row 4 counting from 1.
  const geometry = { screenWidth: 800, screenHeight: 480, cellWidth: 10, cellHeight: 20 };
  const at = { x: 25, y: 70 };

  function sent(setup: string, ...inputs: MouseInput[]): string[] {
    const encoder = new MouseEncoder(vt);
    encoder.setGeometry(geometry);
    const out: string[] = [];
    withTerminal(setup, (term) => {
      for (const input of inputs) out.push(text(encoder.encode(term, input)));
    });
    encoder.free();
    return out;
  }

  it("sends nothing until the program asks for mouse events", () => {
    expect(sent("", { action: "press", button: "left", ...at })).toEqual([""]);
  });

  it("reports press and release in the SGR format", () => {
    expect(
      sent(
        `${ESC}[?1000h${ESC}[?1006h`,
        { action: "press", button: "left", ...at },
        { action: "release", button: "left", ...at },
        { action: "press", button: "right", mods: Mod.Shift, ...at },
      ),
    ).toEqual([`${ESC}[<0;3;4M`, `${ESC}[<0;3;4m`, `${ESC}[<6;3;4M`]);
  });

  it("reports wheel steps as buttons four and five", () => {
    expect(
      sent(
        `${ESC}[?1000h${ESC}[?1006h`,
        { action: "press", button: "four", ...at },
        { action: "press", button: "five", ...at },
      ),
    ).toEqual([`${ESC}[<64;3;4M`, `${ESC}[<65;3;4M`]);
  });

  it("reports motion once per cell when the program asks for all motion", () => {
    expect(
      sent(
        `${ESC}[?1003h${ESC}[?1006h`,
        { action: "motion", x: 25, y: 70 },
        { action: "motion", x: 27, y: 72 },
        { action: "motion", x: 35, y: 70 },
      ),
    ).toEqual([`${ESC}[<35;3;4M`, "", `${ESC}[<35;4;4M`]);
  });

  it("reports motion only while a button is held in button-event mode", () => {
    expect(
      sent(
        `${ESC}[?1002h${ESC}[?1006h`,
        { action: "motion", x: 25, y: 70 },
        { action: "motion", button: "left", anyButtonPressed: true, x: 45, y: 70 },
      ),
    ).toEqual(["", `${ESC}[<32;5;4M`]);
  });

  it("reports pixels in the pixel format", () => {
    expect(sent(`${ESC}[?1000h${ESC}[?1016h`, { action: "press", button: "left", ...at })).toEqual([
      `${ESC}[<0;25;70M`,
    ]);
  });

  it("uses the original byte format by default", () => {
    // Button, column and row are each sent as one byte offset by 32.
    expect(sent(`${ESC}[?1000h`, { action: "press", button: "left", ...at })).toEqual([
      `${ESC}[M${String.fromCharCode(32, 32 + 3, 32 + 4)}`,
    ]);
  });

  it("follows the program when it changes modes between events", () => {
    const encoder = new MouseEncoder(vt);
    encoder.setGeometry(geometry);
    withTerminal(`${ESC}[?1000h${ESC}[?1006h`, (term) => {
      const press: MouseInput = { action: "press", button: "left", ...at };
      expect(text(encoder.encode(term, press))).toBe(`${ESC}[<0;3;4M`);
      term.write(`${ESC}[?1000l`);
      expect(text(encoder.encode(term, press))).toBe("");
    });
    encoder.free();
  });

  it("needs a geometry before the first event", () => {
    const encoder = new MouseEncoder(vt);
    withTerminal("", (term) => {
      expect(() => encoder.encode(term, { action: "press", button: "left", ...at })).toThrow(
        /geometry/,
      );
    });
    encoder.free();
  });
});
