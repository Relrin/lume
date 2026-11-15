import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";

import { beforeAll, describe, expect, it } from "vitest";

import {
  CellFlag,
  CursorShape,
  DirtyState,
  GLYPH_CLUSTER_TAG,
  parseScript,
  RenderState,
  rgba,
  UNDERLINE_MASK,
  UNDERLINE_SHIFT,
  type Color,
  type RenderFrame,
  type Rgb,
  type Terminal,
  type Vt,
} from "../src/index.ts";
import { corpusDir, loadVt } from "./support.ts";

let vt: Vt;
beforeAll(async () => {
  vt = await loadVt();
});

const ESC = "\x1b";

// A palette whose every entry is distinct and recognisable, so a wrong index shows.
const palette = Array.from({ length: 256 }, (_, i): Rgb => ({
  r: i,
  g: 255 - i,
  b: (i * 7) & 0xff,
}));
const foreground: Rgb = { r: 200, g: 210, b: 220 };
const background: Rgb = { r: 10, g: 20, b: 30 };
const colors = { foreground, background, palette };
const pack = (c: Rgb) => rgba(c.r, c.g, c.b);

function resolve(color: Color, fallback: number): number {
  if (color.kind === "palette") return pack(palette[color.index]!);
  if (color.kind === "rgb") return rgba(color.r, color.g, color.b);
  return fallback;
}

function mix(a: number, b: number): number {
  const channel = (shift: number) =>
    Math.round((((a >>> shift) & 0xff) + ((b >>> shift) & 0xff)) / 2);
  return rgba(channel(0), channel(8), channel(16));
}

function glyphText(frame: RenderFrame, index: number): string {
  const glyph = frame.glyph[index]!;
  if (glyph === 0) return "";
  if ((glyph & GLYPH_CLUSTER_TAG) !== 0) return frame.clusters[glyph & ~GLYPH_CLUSTER_TAG]!;
  return String.fromCodePoint(glyph);
}

function rowText(frame: RenderFrame, row: number): string {
  let text = "";
  for (let col = 0; col < frame.cols; col++) {
    const index = row * frame.cols + col;
    if ((frame.flags[index]! & CellFlag.Spacer) !== 0) continue;
    text += glyphText(frame, index) || " ";
  }
  return text.trimEnd();
}

/** Run `body` with a terminal and its render state, both freed afterwards. */
function withState(
  cols: number,
  rows: number,
  body: (term: Terminal, state: RenderState) => void,
): void {
  const term = vt.createTerminal(cols, rows, { colors, scrollbackMaxLines: 1000 });
  const state = new RenderState(term);
  try {
    body(term, state);
  } finally {
    state.free();
    term.free();
  }
}

describe("a frame against the cell-by-cell reader", async () => {
  const names = (await readdir(corpusDir, { withFileTypes: true }))
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();

  it.each(names)("%s reads the same text, attributes and colours", async (name) => {
    const dir = join(corpusDir, name);
    const script = parseScript(await readFile(join(dir, "script.toml"), "utf8"));
    const input = await readFile(join(dir, "input.bin"));

    withState(script.cols, script.rows, (term, state) => {
      let fed = 0;
      for (const r of script.resize) {
        term.write(input.subarray(fed, r.at));
        term.resize(r.cols, r.rows);
        fed = r.at;
      }
      term.write(input.subarray(fed));

      const frame = state.update();
      expect([frame.cols, frame.rows]).toEqual([term.cols, term.rows]);
      const defaultFg = pack(foreground);
      const defaultBg = pack(background);
      let styled = 0;

      for (let row = 0; row < term.rows; row++) {
        for (let col = 0; col < term.cols; col++) {
          const cell = term.cell(col, row);
          const index = row * term.cols + col;
          const flags = frame.flags[index]!;
          const where = `${name} (${col}, ${row})`;

          expect((flags & CellFlag.Spacer) !== 0, where).toBe(cell.wide === "spacer-tail");
          if (cell.wide === "spacer-tail") continue;
          expect(glyphText(frame, index), where).toBe(cell.text);
          expect((flags & CellFlag.Wide) !== 0, where).toBe(cell.wide === "wide");
          expect((flags & CellFlag.Bold) !== 0, where).toBe(cell.attrs.bold);
          expect((flags & CellFlag.Italic) !== 0, where).toBe(cell.attrs.italic);
          expect((flags & CellFlag.Strikethrough) !== 0, where).toBe(cell.attrs.strikethrough);
          expect((flags >> UNDERLINE_SHIFT) & UNDERLINE_MASK, where).toBe(cell.attrs.underline);

          // The frame's colours are the cell's, after the palette, reverse video and
          // faint have been applied.
          let fg = resolve(cell.fg, defaultFg);
          let bg = resolve(cell.bg, 0);
          if (cell.attrs.inverse) [fg, bg] = [bg === 0 ? defaultBg : bg, fg];
          if (cell.attrs.faint) fg = mix(fg, bg === 0 ? defaultBg : bg);
          expect(frame.fg[index], where).toBe(fg);
          // A blank cell may carry a background of its own that the style does not show.
          if (cell.text !== "" || bg !== 0) expect(frame.bg[index], where).toBe(bg);
          if (cell.fg.kind !== "default" || cell.bg.kind !== "default") styled++;
        }
      }
      if (name.includes("colors")) expect(styled).toBeGreaterThan(0);
    });
  });
});

describe("colour resolution", () => {
  it("gives erased cells the background they were erased with", () => {
    withState(10, 2, (term, state) => {
      term.write(`${ESC}[41m${ESC}[K${ESC}[48;2;1;2;3m${ESC}[2;1H${ESC}[K`);
      const frame = state.update();
      expect(frame.glyph[3]).toBe(0);
      expect(frame.bg[3]).toBe(pack(palette[1]!));
      expect(frame.bg[10 + 3]).toBe(rgba(1, 2, 3));
    });
  });

  it("leaves the default background transparent and the default foreground opaque", () => {
    withState(4, 1, (term, state) => {
      term.write("a");
      const frame = state.update();
      expect(frame.bg[0]).toBe(0);
      expect(frame.fg[0]).toBe(pack(foreground));
      expect(frame.foreground).toBe(pack(foreground));
      expect(frame.background).toBe(pack(background));
    });
  });

  it("swaps the pair for reverse video, using the default background as text colour", () => {
    withState(4, 1, (term, state) => {
      term.write(`${ESC}[7ma${ESC}[0;7;32;44mb`);
      const frame = state.update();
      expect([frame.fg[0], frame.bg[0]]).toEqual([pack(background), pack(foreground)]);
      expect([frame.fg[1], frame.bg[1]]).toEqual([pack(palette[4]!), pack(palette[2]!)]);
    });
  });

  it("mixes faint text halfway to the background", () => {
    withState(4, 1, (term, state) => {
      term.write(`${ESC}[2;38;2;200;100;0;48;2;0;100;200ma`);
      const frame = state.update();
      expect(frame.fg[0]).toBe(rgba(100, 100, 100));
    });
  });

  it("hides concealed text and its decorations but keeps the background", () => {
    withState(4, 1, (term, state) => {
      term.write(`${ESC}[8;4;9;41ma`);
      const frame = state.update();
      expect(frame.glyph[0]).toBe(0);
      expect(frame.flags[0]! & (CellFlag.Strikethrough | (UNDERLINE_MASK << UNDERLINE_SHIFT))).toBe(
        0,
      );
      expect(frame.bg[0]).toBe(pack(palette[1]!));
    });
  });

  it("uses the text colour for an underline unless one was set", () => {
    withState(4, 1, (term, state) => {
      term.write(`${ESC}[4;31ma${ESC}[58;2;9;8;7mb${ESC}[58;5;200mc`);
      const frame = state.update();
      expect(frame.underlineColor[0]).toBe(pack(palette[1]!));
      expect(frame.underlineColor[1]).toBe(rgba(9, 8, 7));
      expect(frame.underlineColor[2]).toBe(pack(palette[200]!));
    });
  });

  it("re-reads every row when a default colour changes", () => {
    withState(4, 2, (term, state) => {
      term.write("a");
      state.update();
      state.markDrawn();
      term.write(`${ESC}]10;rgb:11/22/33${ESC}\\`);
      const frame = state.update();
      expect(frame.dirty).toBe(DirtyState.Full);
      expect(frame.fg[0]).toBe(rgba(0x11, 0x22, 0x33));
    });
  });
});

describe("text", () => {
  it("keeps a combining sequence together as one cell", () => {
    withState(6, 1, (term, state) => {
      term.write("éx");
      const frame = state.update();
      expect(frame.glyph[0]! & GLYPH_CLUSTER_TAG).not.toBe(0);
      expect(glyphText(frame, 0)).toBe("é");
      expect(frame.glyph[1]).toBe(0x78);
    });
  });

  it("gives the same cluster the same entry and forgets clusters on a full read", () => {
    withState(6, 2, (term, state) => {
      term.write("éé\r\nä");
      const frame = state.update();
      expect(frame.glyph[0]).toBe(frame.glyph[1]);
      expect(frame.clusters).toEqual(["é", "ä"]);

      term.write(`${ESC}[2J${ESC}[Hô`);
      state.update();
      term.resize(7, 2);
      state.update();
      expect(frame.clusters).toEqual(["ô"]);
    });
  });

  it("marks both halves of a wide character and copies the style to the second", () => {
    withState(6, 1, (term, state) => {
      term.write(`${ESC}[4;44m中`);
      const frame = state.update();
      expect(frame.glyph[0]).toBe(0x4e2d);
      expect(frame.flags[0]! & CellFlag.Wide).not.toBe(0);
      expect(frame.glyph[1]).toBe(0);
      expect(frame.flags[1]! & CellFlag.Spacer).not.toBe(0);
      expect((frame.flags[1]! >> UNDERLINE_SHIFT) & UNDERLINE_MASK).toBe(1);
      expect(frame.bg[1]).toBe(frame.bg[0]);
    });
  });

  it("flags the cells of a hyperlink", () => {
    withState(8, 1, (term, state) => {
      term.write(`a${ESC}]8;;https://example.com${ESC}\\bc${ESC}]8;;${ESC}\\d`);
      const frame = state.update();
      const linked = [0, 1, 2, 3].map((i) => (frame.flags[i]! & CellFlag.Hyperlink) !== 0);
      expect(linked).toEqual([false, true, true, false]);
    });
  });
});

describe("dirty tracking", () => {
  it("starts with everything dirty and reports nothing after an idle update", () => {
    withState(10, 4, (term, state) => {
      term.write("hello");
      const frame = state.update();
      expect(frame.dirty).toBe(DirtyState.Full);
      expect([...frame.dirtyRows]).toEqual([1, 1, 1, 1]);
      state.markDrawn();
      state.update();
      expect(frame.dirty).toBe(DirtyState.None);
      expect([...frame.dirtyRows]).toEqual([0, 0, 0, 0]);
    });
  });

  it("flags only the rows that changed", () => {
    withState(10, 4, (term, state) => {
      term.write("one\r\ntwo\r\nthree");
      state.update();
      state.markDrawn();
      term.write(`${ESC}[2;1HTWO`);
      const frame = state.update();
      expect(frame.dirty).toBe(DirtyState.Partial);
      expect(frame.dirtyRows[1]).toBe(1);
      expect(frame.dirtyRows[3]).toBe(0);
      expect(rowText(frame, 1)).toBe("TWO");
      expect(rowText(frame, 0)).toBe("one");
    });
  });

  it("keeps dirty rows until the frame is marked drawn", () => {
    withState(10, 4, (term, state) => {
      state.update();
      state.markDrawn();
      term.write("a");
      state.update();
      term.write(`${ESC}[3;1Hb`);
      const frame = state.update();
      expect(frame.dirtyRows[0]).toBe(1);
      expect(frame.dirtyRows[2]).toBe(1);
    });
  });

  it("reports a cursor that only moved", () => {
    withState(10, 4, (term, state) => {
      term.write("abc");
      state.update();
      state.markDrawn();
      term.write(`${ESC}[D`);
      const frame = state.update();
      expect(frame.dirty).not.toBe(DirtyState.None);
      expect(frame.cursor.x).toBe(2);
    });
  });

  it("replaces the arrays and redraws everything after a resize", () => {
    withState(10, 4, (term, state) => {
      term.write("abc");
      const before = state.update().glyph;
      state.markDrawn();
      term.resize(20, 6);
      const frame = state.update();
      expect(frame.glyph).not.toBe(before);
      expect(frame.glyph.length).toBe(120);
      expect(frame.dirty).toBe(DirtyState.Full);
      expect(rowText(frame, 0)).toBe("abc");
    });
  });
});

describe("the cursor", () => {
  it("follows the program's position, shape and visibility", () => {
    withState(10, 4, (term, state) => {
      term.write(`ab${ESC}[5 q`);
      const frame = state.update();
      expect(frame.cursor).toMatchObject({
        visible: true,
        x: 2,
        y: 0,
        shape: CursorShape.Bar,
        blinking: true,
      });
      term.write(`${ESC}[4 q`);
      state.update();
      expect(frame.cursor).toMatchObject({ shape: CursorShape.Underline, blinking: false });
      term.write(`${ESC}[?25l`);
      state.update();
      expect(frame.cursor.visible).toBe(false);
    });
  });

  it("is not drawn while the viewport is scrolled away from it", () => {
    withState(10, 4, (term, state) => {
      term.write(Array.from({ length: 30 }, (_, i) => `l${i}`).join("\r\n"));
      expect(state.update().cursor.visible).toBe(true);
      term.scrollToTop();
      const frame = state.update();
      expect(frame.cursor.visible).toBe(false);
      expect(rowText(frame, 0)).toBe("l0");
      term.scrollToBottom();
      expect(state.update().cursor.visible).toBe(true);
      expect(rowText(frame, 3)).toBe("l29");
    });
  });

  it("carries a colour the program set", () => {
    withState(10, 4, (term, state) => {
      expect(state.update().cursor.hasColor).toBe(false);
      term.write(`${ESC}]12;rgb:01/02/03${ESC}\\`);
      const frame = state.update();
      expect(frame.cursor.hasColor).toBe(true);
      expect(frame.cursor.color).toBe(rgba(1, 2, 3));
    });
  });
});

describe("the default cursor", () => {
  /** Run `body` with a terminal created with `options` and its render state. */
  function withCursor(
    options: Parameters<Vt["createTerminal"]>[2],
    body: (term: Terminal, state: RenderState) => void,
  ): void {
    const term = vt.createTerminal(10, 4, options);
    const state = new RenderState(term);
    try {
      body(term, state);
    } finally {
      state.free();
      term.free();
    }
  }

  it("is a block that does not blink unless the terminal is created with another", () => {
    withCursor({}, (_term, state) => {
      expect(state.update().cursor).toMatchObject({ shape: CursorShape.Block, blinking: false });
    });
    withCursor({ defaultCursorStyle: "bar", defaultCursorBlink: true }, (_term, state) => {
      expect(state.update().cursor).toMatchObject({ shape: CursorShape.Bar, blinking: true });
    });
    withCursor({ defaultCursorStyle: "underline" }, (_term, state) => {
      expect(state.update().cursor).toMatchObject({
        shape: CursorShape.Underline,
        blinking: false,
      });
    });
    withCursor({ defaultCursorBlink: true }, (_term, state) => {
      expect(state.update().cursor).toMatchObject({ shape: CursorShape.Block, blinking: true });
    });
  });

  it("changes at once while the program has left the cursor alone", () => {
    withCursor({}, (term, state) => {
      term.write("ab");
      const frame = state.update();
      state.markDrawn();
      term.setDefaultCursor("underline", true);
      state.update();
      expect(frame.dirty).not.toBe(DirtyState.None);
      expect(frame.cursor).toMatchObject({
        x: 2,
        shape: CursorShape.Underline,
        blinking: true,
      });
      term.setDefaultCursor("block", false);
      state.update();
      expect(frame.cursor).toMatchObject({ shape: CursorShape.Block, blinking: false });
    });
  });

  it("leaves a shape the program chose until the program resets it", () => {
    withCursor({ defaultCursorStyle: "bar", defaultCursorBlink: true }, (term, state) => {
      // A steady underline, by the program's choice.
      term.write(`${ESC}[4 q`);
      const frame = state.update();
      expect(frame.cursor).toMatchObject({ shape: CursorShape.Underline, blinking: false });

      term.setDefaultCursor("block", true);
      state.update();
      expect(frame.cursor).toMatchObject({ shape: CursorShape.Underline, blinking: false });

      term.write(`${ESC}[0 q`);
      state.update();
      expect(frame.cursor).toMatchObject({ shape: CursorShape.Block, blinking: true });
    });
  });

  it("is what a program's reset of the cursor's shape comes back to", () => {
    withCursor({ defaultCursorStyle: "bar", defaultCursorBlink: true }, (term, state) => {
      term.write(`${ESC}[2 q`);
      const frame = state.update();
      expect(frame.cursor).toMatchObject({ shape: CursorShape.Block, blinking: false });
      term.write(`${ESC}[0 q`);
      state.update();
      expect(frame.cursor).toMatchObject({ shape: CursorShape.Bar, blinking: true });
    });
  });

  it("is kept across a full reset", () => {
    withCursor({}, (term, state) => {
      term.setDefaultCursor("bar", true);
      term.write(`${ESC}[2 q`);
      term.reset();
      const frame = state.update();
      expect(frame.cursor).toMatchObject({ shape: CursorShape.Bar, blinking: true });
      term.write(`${ESC}[4 q${ESC}c`);
      state.update();
      expect(frame.cursor).toMatchObject({ shape: CursorShape.Bar, blinking: true });
    });
  });
});

describe("colours changed on a live terminal", () => {
  const newForeground: Rgb = { r: 1, g: 2, b: 3 };
  const newBackground: Rgb = { r: 250, g: 240, b: 230 };
  const newCursor: Rgb = { r: 9, g: 99, b: 199 };
  const newPalette = palette.map((c): Rgb => ({ r: c.g, g: c.b, b: c.r }));

  it("reach the next frame, with every row to be drawn again", () => {
    withState(6, 3, (term, state) => {
      // Default colours on the first row, palette colours on the second.
      term.write(`ab\r\n${ESC}[31;44mcd${ESC}[0m`);
      const frame = state.update();
      state.markDrawn();
      expect(frame.foreground).toBe(pack(foreground));
      expect(frame.background).toBe(pack(background));
      expect(frame.fg[6]).toBe(pack(palette[1]!));
      expect(frame.bg[6]).toBe(pack(palette[4]!));
      expect(frame.cursor.hasColor).toBe(false);
      expect(state.update().dirty).toBe(DirtyState.None);

      term.setColors({
        foreground: newForeground,
        background: newBackground,
        cursor: newCursor,
        palette: newPalette,
      });
      state.update();
      expect(frame.dirty).toBe(DirtyState.Full);
      expect(frame.foreground).toBe(pack(newForeground));
      expect(frame.background).toBe(pack(newBackground));
      expect(frame.fg[0]).toBe(pack(newForeground));
      expect(frame.fg[6]).toBe(pack(newPalette[1]!));
      expect(frame.bg[6]).toBe(pack(newPalette[4]!));
      expect(frame.cursor.hasColor).toBe(true);
      expect(frame.cursor.color).toBe(pack(newCursor));
    });
  });

  it("change only the colours that are given", () => {
    withState(6, 3, (term, state) => {
      term.write(`${ESC}[32ma`);
      const frame = state.update();
      state.markDrawn();
      term.setColors({ background: newBackground });
      state.update();
      expect(frame.dirty).toBe(DirtyState.Full);
      expect(frame.background).toBe(pack(newBackground));
      expect(frame.foreground).toBe(pack(foreground));
      expect(frame.fg[0]).toBe(pack(palette[2]!));
    });
  });

  it("are what a program is told when it asks afterwards", () => {
    withState(6, 3, (term) => {
      const replies: string[] = [];
      const decoder = new TextDecoder();
      term.handlers = { output: (bytes) => replies.push(decoder.decode(bytes)) };
      term.setColors({ foreground: newForeground, palette: newPalette });
      term.write(`${ESC}]10;?${ESC}\\${ESC}]4;1;?${ESC}\\`);
      const one = newPalette[1]!;
      const hex = (value: number) => value.toString(16).padStart(2, "0").repeat(2);
      expect(replies).toEqual([
        `${ESC}]10;rgb:0101/0202/0303${ESC}\\`,
        `${ESC}]4;1;rgb:${hex(one.r)}/${hex(one.g)}/${hex(one.b)}${ESC}\\`,
      ]);
    });
  });
});

describe("lifetime", () => {
  it("refuses use after free and frees twice safely", () => {
    const term = vt.createTerminal(4, 2);
    const state = new RenderState(term);
    state.free();
    state.free();
    expect(() => state.update()).toThrow(/freed/);
    term.free();
  });

  it("does not grow memory across many frames of a changing screen", () => {
    withState(120, 40, (term, state) => {
      const line = `${ESC}[3${1}mcolour ${ESC}[1mbold${ESC}[0m é 中文 plain text to fill the row\r\n`;
      for (let i = 0; i < 20; i++) {
        term.write(line.repeat(40));
        state.update();
        state.markDrawn();
      }
      const before = vt.core.x.memory.buffer.byteLength;
      for (let i = 0; i < 200; i++) {
        term.write(line.repeat(40));
        state.update();
        state.markDrawn();
      }
      expect(vt.core.x.memory.buffer.byteLength).toBe(before);
    });
  });
});
