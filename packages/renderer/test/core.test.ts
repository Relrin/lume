import { describe, expect, it } from "vitest";

import { ATLAS_PAGE_SIZE, LINE_RECTS_PER_CELL } from "../src/constants.ts";
import { GridRenderer } from "../src/core.ts";
import { CursorShape, rgba, UnderlineStyle, type RenderFrame } from "../src/frame.ts";
import { GLYPH, GLYPH_FLAG_COLOR, RECT, type Plane } from "../src/layout.ts";
import {
  FakeFonts,
  FIXED_FRAMES,
  makeFrame,
  markDrawn,
  PALETTE,
  putText,
  RecordingBackend,
  select,
  setCursor,
  TEST_THEME,
  touchRow,
} from "../src/testing/index.ts";
import type { TerminalTheme } from "../src/types.ts";

// The fake font gives cells of 8 by 16 pixels at a pixel ratio of 1.
const CW = 8;
const CH = 16;

function setup(theme: TerminalTheme = TEST_THEME) {
  const backend = new RecordingBackend();
  const fonts = new FakeFonts();
  const renderer = new GridRenderer(backend, fonts, theme);
  renderer.resize({ width: 640, height: 384 }, 1);
  return { backend, fonts, renderer };
}

/** Draw a frame and mark it drawn, as a view does every animation frame. */
function drawn(renderer: GridRenderer, frame: RenderFrame): void {
  renderer.draw(frame);
  markDrawn(frame);
}

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
  color: number;
}

function rect(plane: Plane, slot: number): Rect | undefined {
  const view = new DataView(plane.bytes.buffer, slot * RECT.stride, RECT.stride);
  const w = view.getUint16(4, true);
  if (w === 0) return undefined;
  return {
    x: view.getInt16(0, true),
    y: view.getInt16(2, true),
    w,
    h: view.getUint16(6, true),
    color: view.getUint32(8, true),
  };
}

interface Glyph extends Rect {
  u: number;
  v: number;
  page: number;
  flags: number;
}

function glyph(plane: Plane, slot: number): Glyph | undefined {
  const view = new DataView(plane.bytes.buffer, slot * GLYPH.stride, GLYPH.stride);
  const w = view.getUint16(4, true);
  if (w === 0) return undefined;
  return {
    x: view.getInt16(0, true),
    y: view.getInt16(2, true),
    w,
    h: view.getUint16(6, true),
    u: view.getUint16(8, true),
    v: view.getUint16(10, true),
    page: view.getUint16(12, true),
    flags: view.getUint16(14, true),
    color: view.getUint32(16, true),
  };
}

describe("building the records of a frame", () => {
  it("places a glyph in its cell at the offset the atlas gives", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 3, 2, "A", { fg: PALETTE[1]! });
    renderer.draw(frame);
    const planes = backend.planes!;
    const at = 2 * 80 + 3;
    // The fake rasteriser draws a box inset by one pixel across and two down.
    expect(glyph(planes.glyph, at)).toMatchObject({
      x: 3 * CW + 1,
      y: 2 * CH + 2,
      w: CW - 2,
      h: CH - 4,
      color: PALETTE[1],
      flags: 0,
    });
    expect(glyph(planes.glyph, at + 1)).toBeUndefined();
  });

  it("draws a background only where the cell has one", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 0, 0, "ab", { bg: PALETTE[4]! });
    putText(frame, 2, 0, "c");
    renderer.draw(frame);
    const planes = backend.planes!;
    expect(rect(planes.background, 0)).toEqual({ x: 0, y: 0, w: CW, h: CH, color: PALETTE[4] });
    expect(rect(planes.background, 1)).toMatchObject({ x: CW });
    expect(rect(planes.background, 2)).toBeUndefined();
  });

  it("offsets everything by the padding", () => {
    const { renderer, backend } = setup({ ...TEST_THEME, padding: 5 });
    const frame = makeFrame(40, 10);
    putText(frame, 1, 1, "A", { bg: PALETTE[2]! });
    renderer.draw(frame);
    expect(rect(backend.planes!.background, 41)).toMatchObject({ x: 5 + CW, y: 5 + CH });
  });

  it("gives a wide character one glyph two cells wide and none for its second cell", () => {
    const { renderer, backend, fonts } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 0, 0, "漢", { bg: PALETTE[3]! });
    renderer.draw(frame);
    const planes = backend.planes!;
    expect(glyph(planes.glyph, 0)).toMatchObject({ x: 1, w: 2 * CW - 2 });
    expect(glyph(planes.glyph, 1)).toBeUndefined();
    expect(rect(planes.background, 0)).toMatchObject({ x: 0, w: CW });
    expect(rect(planes.background, 1)).toMatchObject({ x: CW, w: CW });
    expect(fonts.rasterisers[0]!.calls).toEqual(["漢 x2"]);
  });

  it("flags a colour glyph so it is not tinted", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 0, 0, "😀");
    renderer.draw(frame);
    expect(glyph(backend.planes!.glyph, 0)).toMatchObject({ flags: GLYPH_FLAG_COLOR });
  });

  it("draws clusters through their text", () => {
    const { renderer, backend, fonts } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 0, 0, "éé");
    renderer.draw(frame);
    const planes = backend.planes!;
    expect(glyph(planes.glyph, 0)).toMatchObject({ u: glyph(planes.glyph, 1)!.u });
    expect(fonts.rasterisers[0]!.calls).toEqual(["é x1"]);
  });

  it("puts an underline sprite under the text in the underline colour", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 0, 0, "a", { underline: UnderlineStyle.Single, underlineColor: PALETTE[9]! });
    putText(frame, 1, 0, " ", { underline: UnderlineStyle.Curly });
    renderer.draw(frame);
    const planes = backend.planes!;
    const m = renderer.metrics;
    expect(glyph(planes.underline, 0)).toMatchObject({
      x: 0,
      y: m.underlineTop,
      w: CW,
      h: m.lineThickness,
      color: PALETTE[9],
    });
    // An underlined blank still has its underline.
    expect(glyph(planes.underline, 1)).toMatchObject({ x: CW, w: CW });
    expect(glyph(planes.underline, 2)).toBeUndefined();
  });

  it("draws strikethrough and overline as bars in the text colour", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 2, 1, "a", { strikethrough: true, overline: true, fg: PALETTE[5]! });
    renderer.draw(frame);
    const planes = backend.planes!;
    const m = renderer.metrics;
    const slot = (80 + 2) * LINE_RECTS_PER_CELL;
    expect(rect(planes.line, slot)).toEqual({
      x: 2 * CW,
      y: CH + m.strikethroughTop,
      w: CW,
      h: m.lineThickness,
      color: PALETTE[5],
    });
    expect(rect(planes.line, slot + 1)).toMatchObject({ y: CH, h: m.lineThickness });
  });

  it("recolours selected cells", () => {
    const theme = { ...TEST_THEME, selectionForeground: rgba(1, 2, 3) };
    const { renderer, backend } = setup(theme);
    const frame = makeFrame(80, 24);
    putText(frame, 0, 0, "abcd", { fg: PALETTE[1]!, bg: PALETTE[2]! });
    select(frame, 0, 1, 2);
    renderer.draw(frame);
    const planes = backend.planes!;
    expect(rect(planes.background, 0)!.color).toBe(PALETTE[2]);
    expect(rect(planes.background, 1)!.color).toBe(theme.selectionBackground);
    expect(rect(planes.background, 2)!.color).toBe(theme.selectionBackground);
    expect(rect(planes.background, 3)!.color).toBe(PALETTE[2]);
    expect(glyph(planes.glyph, 0)!.color).toBe(PALETTE[1]);
    expect(glyph(planes.glyph, 1)!.color).toBe(rgba(1, 2, 3));
  });

  it("keeps the text colour in a selection when the theme sets none", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 0, 0, "ab", { fg: PALETTE[1]! });
    select(frame, 0, 0, 1);
    renderer.draw(frame);
    expect(glyph(backend.planes!.glyph, 0)!.color).toBe(PALETTE[1]);
    // A selected cell with no background of its own still gets the selection colour.
    expect(rect(backend.planes!.background, 1)!.color).toBe(TEST_THEME.selectionBackground);
  });

  it("builds every frame of the fixed set", () => {
    for (const fixed of FIXED_FRAMES) {
      const { renderer, backend } = setup();
      renderer.draw(fixed.build());
      expect(backend.presents.length, fixed.name).toBe(1);
    }
  });
});

describe("the cursor", () => {
  function cursorRects(shape: number, focused = true) {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 4, 2, "Q");
    setCursor(frame, 4, 2, shape);
    renderer.setFocused(focused);
    renderer.draw(frame);
    const planes = backend.planes!;
    return {
      renderer,
      backend,
      rects: [0, 1, 2, 3].map((i) => rect(planes.overlayRect, i)).filter((r) => r !== undefined),
      glyph: glyph(planes.overlayGlyph, 0),
    };
  }

  it("covers its cell as a block and redraws the glyph on top", () => {
    const { rects, glyph: over, backend } = cursorRects(CursorShape.Block);
    expect(rects).toEqual([{ x: 4 * CW, y: 2 * CH, w: CW, h: CH, color: TEST_THEME.cursor }]);
    // In the default background colour, so it shows against the cursor.
    expect(over).toMatchObject({ x: 4 * CW + 1, color: makeFrame(1, 1).background });
    expect([backend.last.overlayRects, backend.last.overlayGlyphs]).toEqual([1, 1]);
  });

  it("is a thin bar at the left of the cell", () => {
    const { rects, glyph: over } = cursorRects(CursorShape.Bar);
    expect(rects).toEqual([{ x: 4 * CW, y: 2 * CH, w: 1, h: CH, color: TEST_THEME.cursor }]);
    expect(over).toBeUndefined();
  });

  it("is a bar along the bottom of the cell", () => {
    const { rects, renderer } = cursorRects(CursorShape.Underline);
    const t = renderer.metrics.lineThickness;
    expect(rects).toEqual([
      { x: 4 * CW, y: 2 * CH + CH - t, w: CW, h: t, color: TEST_THEME.cursor },
    ]);
  });

  it("is an outline when hollow, whose four sides do not overlap", () => {
    const { rects } = cursorRects(CursorShape.HollowBlock);
    expect(rects.length).toBe(4);
    const area = rects.reduce((sum, r) => sum + r.w * r.h, 0);
    const t = 1;
    expect(area).toBe(2 * CW * t + 2 * (CH - 2 * t) * t);
  });

  it("is hollow in every shape while the terminal lacks focus", () => {
    expect(cursorRects(CursorShape.Bar, false).rects.length).toBe(4);
    expect(cursorRects(CursorShape.Block, false).glyph).toBeUndefined();
  });

  it("covers both cells of a wide character, from either half", () => {
    for (const [x, wideTail] of [
      [2, false],
      [3, true],
    ] as const) {
      const { renderer, backend } = setup();
      const frame = makeFrame(80, 24);
      putText(frame, 2, 0, "漢");
      setCursor(frame, x, 0, CursorShape.Block);
      frame.cursor.wideTail = wideTail;
      renderer.draw(frame);
      expect(rect(backend.planes!.overlayRect, 0)).toMatchObject({ x: 2 * CW, w: 2 * CW });
      expect(glyph(backend.planes!.overlayGlyph, 0)).toMatchObject({ w: 2 * CW - 2 });
    }
  });

  it("uses the colour a program set", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    setCursor(frame, 0, 0, CursorShape.Block);
    Object.assign(frame.cursor, { hasColor: true, color: rgba(9, 8, 7) });
    renderer.draw(frame);
    expect(rect(backend.planes!.overlayRect, 0)!.color).toBe(rgba(9, 8, 7));
  });

  it("draws nothing when hidden", () => {
    const { renderer, backend } = setup();
    renderer.draw(makeFrame(80, 24));
    expect([backend.last.overlayRects, backend.last.overlayGlyphs]).toEqual([0, 0]);
  });

  it("disappears and comes back when a blink turns it off and on, repainting only its row", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 4, 2, "Q");
    setCursor(frame, 4, 2, CursorShape.Block);
    drawn(renderer, frame);
    backend.clear();

    renderer.setCursorVisible(false);
    renderer.draw(frame);
    expect([backend.last.overlayRects, backend.last.overlayGlyphs]).toEqual([0, 0]);
    expect(rect(backend.planes!.overlayRect, 0)).toBeUndefined();
    expect(backend.uploads.map((u) => u.plane)).toEqual(["overlayRect", "overlayGlyph"]);
    expect([...backend.last.repaint!].flatMap((dirty, row) => (dirty ? [row] : []))).toEqual([2]);

    backend.clear();
    renderer.setCursorVisible(true);
    renderer.draw(frame);
    expect([backend.last.overlayRects, backend.last.overlayGlyphs]).toEqual([1, 1]);
    expect([...backend.last.repaint!].flatMap((dirty, row) => (dirty ? [row] : []))).toEqual([2]);

    // Setting it to what it already is draws nothing.
    backend.clear();
    renderer.setCursorVisible(true);
    renderer.draw(frame);
    expect(backend.presents).toEqual([]);
  });
});

describe("underlined runs", () => {
  function underlined(cursor: boolean) {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 10, 3, "https://example.org");
    if (cursor) setCursor(frame, 0, 0, CursorShape.HollowBlock);
    drawn(renderer, frame);
    backend.clear();
    return { renderer, backend, frame };
  }

  it("are drawn at the underline position in the text colour of their first cell", () => {
    const { renderer, backend, frame } = underlined(false);
    renderer.setUnderlines([{ row: 3, startCol: 10, endCol: 28 }]);
    renderer.draw(frame);

    const m = renderer.metrics;
    expect(backend.last.overlayRects).toBe(1);
    expect(rect(backend.planes!.overlayRect, 0)).toEqual({
      x: 10 * CW,
      y: 3 * CH + m.underlineTop,
      w: 19 * CW,
      h: m.lineThickness,
      color: frame.fg[3 * 80 + 10],
    });
    expect([...backend.last.repaint!].flatMap((dirty, row) => (dirty ? [row] : []))).toEqual([3]);
    // The rows under them are not rebuilt.
    expect(backend.uploads.map((u) => u.plane)).toEqual(["overlayRect", "overlayGlyph"]);
  });

  it("follow the cursor's rectangles", () => {
    const { renderer, backend, frame } = underlined(true);
    renderer.setUnderlines([
      { row: 3, startCol: 10, endCol: 79 },
      { row: 4, startCol: 0, endCol: 5 },
    ]);
    renderer.draw(frame);
    expect(backend.last.overlayRects).toBe(6);
    expect(rect(backend.planes!.overlayRect, 4)).toMatchObject({ x: 10 * CW, w: 70 * CW });
    expect(rect(backend.planes!.overlayRect, 5)).toMatchObject({ x: 0, w: 6 * CW });
  });

  it("go away again, repainting the rows they were on", () => {
    const { renderer, backend, frame } = underlined(false);
    renderer.setUnderlines([{ row: 3, startCol: 10, endCol: 28 }]);
    drawn(renderer, frame);
    backend.clear();

    renderer.setUnderlines([]);
    renderer.draw(frame);
    expect(backend.last.overlayRects).toBe(0);
    expect([...backend.last.repaint!].flatMap((dirty, row) => (dirty ? [row] : []))).toEqual([3]);
  });

  it("are cut to the grid, and those outside it or beyond the limit are left out", () => {
    const { renderer, backend, frame } = underlined(false);
    renderer.setUnderlines([
      { row: 5, startCol: -3, endCol: 200 },
      { row: 99, startCol: 0, endCol: 5 },
      { row: 6, startCol: 9, endCol: 4 },
      ...Array.from({ length: 20 }, (_, i) => ({ row: 7 + (i % 10), startCol: 0, endCol: 1 })),
    ]);
    renderer.draw(frame);
    expect(rect(backend.planes!.overlayRect, 0)).toMatchObject({ x: 0, w: 80 * CW });
    expect(backend.last.overlayRects).toBe(backend.planes!.overlayRect.capacity);
  });

  it("are the renderer's own copy, not the caller's list", () => {
    const { renderer, backend, frame } = underlined(false);
    const ranges = [{ row: 3, startCol: 10, endCol: 28 }];
    renderer.setUnderlines(ranges);
    drawn(renderer, frame);
    backend.clear();
    ranges[0]!.row = 9;
    renderer.draw(frame);
    expect(backend.presents).toEqual([]);
  });
});

describe("what reaches the tier", () => {
  it("configures the surface and sends everything for the first frame", () => {
    const { renderer, backend } = setup();
    renderer.draw(makeFrame(80, 24));
    expect(backend.configured).toEqual([[640, 384]]);
    expect(backend.planesSet).toBe(1);
    const planes = backend.planes!;
    for (const name of ["background", "underline", "glyph", "line"] as const) {
      expect(backend.uploads).toContainEqual({
        plane: name,
        start: 0,
        end: planes[name].bytes.byteLength,
      });
    }
    expect(backend.last).toMatchObject({ width: 640, height: 384, cols: 80, rows: 24 });
    expect(backend.last.repaint).toBeUndefined();
  });

  it("does nothing for a frame with no changes", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    drawn(renderer, frame);
    backend.clear();
    renderer.draw(frame);
    expect(backend.uploads).toEqual([]);
    expect(backend.presents).toEqual([]);
  });

  it("sends only the bytes of the rows that changed", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    drawn(renderer, frame);
    backend.clear();

    putText(frame, 0, 5, "x");
    touchRow(frame, 5);
    putText(frame, 0, 6, "y");
    touchRow(frame, 6);
    putText(frame, 0, 20, "z");
    touchRow(frame, 20);
    renderer.draw(frame);

    const rowBytes = 80 * GLYPH.stride;
    expect(backend.uploads.filter((u) => u.plane === "glyph")).toEqual([
      { plane: "glyph", start: 5 * rowBytes, end: 7 * rowBytes },
      { plane: "glyph", start: 20 * rowBytes, end: 21 * rowBytes },
    ]);
    const lineBytes = 80 * LINE_RECTS_PER_CELL * RECT.stride;
    expect(backend.uploads.filter((u) => u.plane === "line")[0]).toEqual({
      plane: "line",
      start: 5 * lineBytes,
      end: 7 * lineBytes,
    });
    expect([...backend.last.repaint!].flatMap((dirty, row) => (dirty ? [row] : []))).toEqual([
      5, 6, 20,
    ]);
  });

  it("sends a whole plane at once when most rows changed", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    drawn(renderer, frame);
    backend.clear();
    for (let row = 0; row < 20; row++) touchRow(frame, row);
    renderer.draw(frame);
    const glyphs = backend.uploads.filter((u) => u.plane === "glyph");
    expect(glyphs).toEqual([
      { plane: "glyph", start: 0, end: backend.planes!.glyph.bytes.byteLength },
    ]);
  });

  it("redraws the rows a moving cursor leaves and enters, without rebuilding them", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    setCursor(frame, 0, 3, CursorShape.Block);
    drawn(renderer, frame);
    backend.clear();

    frame.cursor.y = 9;
    renderer.draw(frame);
    expect(backend.uploads.map((u) => u.plane)).toEqual(["overlayRect", "overlayGlyph"]);
    expect([...backend.last.repaint!].flatMap((dirty, row) => (dirty ? [row] : []))).toEqual([
      3, 9,
    ]);
  });

  it("redraws when focus changes", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    setCursor(frame, 0, 0, CursorShape.Block);
    drawn(renderer, frame);
    backend.clear();
    renderer.setFocused(false);
    renderer.draw(frame);
    expect(backend.presents.length).toBe(1);
    expect(backend.last.overlayRects).toBe(4);
  });

  it("rebuilds rows whose selection changed", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 0, 2, "abc");
    drawn(renderer, frame);
    backend.clear();
    select(frame, 2, 0, 1);
    renderer.draw(frame);
    expect(rect(backend.planes!.background, 2 * 80)!.color).toBe(TEST_THEME.selectionBackground);
    expect([...backend.last.repaint!].flatMap((dirty, row) => (dirty ? [row] : []))).toEqual([2]);
  });

  it("applies a new size with the next frame and redraws everything", () => {
    const { renderer, backend } = setup();
    const frame = makeFrame(80, 24);
    drawn(renderer, frame);
    backend.clear();
    renderer.resize({ width: 800, height: 600 }, 1);
    // Nothing happens until there is a frame to fill the resized surface with.
    expect(backend.configured.length).toBe(1);
    renderer.draw(frame);
    expect(backend.configured.at(-1)).toEqual([800, 600]);
    expect(backend.last.repaint).toBeUndefined();
    expect(renderer.gridSize()).toEqual({ cols: 100, rows: 37 });
  });

  it("does not draw on a surface with no area", () => {
    const { renderer, backend } = setup();
    renderer.resize({ width: 0, height: 100 }, 1);
    renderer.draw(makeFrame(80, 24));
    expect(backend.presents).toEqual([]);
    expect(backend.configured).toEqual([]);
  });

  it("makes new planes when the grid changes size", () => {
    const { renderer, backend } = setup();
    drawn(renderer, makeFrame(80, 24));
    const before = backend.planes;
    renderer.draw(makeFrame(100, 30));
    expect(backend.planesSet).toBe(2);
    expect(backend.planes).not.toBe(before);
    expect(backend.planes!.glyph.capacity).toBe(3000);
    expect(backend.last.repaint).toBeUndefined();
  });

  it("measures the font again and takes a new atlas when the pixel ratio changes", () => {
    const { renderer, backend, fonts } = setup();
    const frame = makeFrame(80, 24);
    putText(frame, 0, 0, "A");
    drawn(renderer, frame);
    expect(renderer.metrics.cellWidth).toBe(8);
    backend.clear();

    renderer.resize({ width: 640, height: 384 }, 2);
    renderer.draw(frame);
    expect(renderer.metrics.cellWidth).toBe(16);
    expect(fonts.measured).toBe(2);
    expect(backend.atlases.size).toBe(2);
    expect(backend.configured.at(-1)).toEqual([1280, 768]);
    expect(glyph(backend.planes!.glyph, 0)).toMatchObject({ w: 14 });
  });

  it("redraws everything after a theme change and re-measures only for a new font", () => {
    const { renderer, backend, fonts } = setup();
    const frame = makeFrame(80, 24);
    setCursor(frame, 0, 0, CursorShape.Bar);
    drawn(renderer, frame);
    backend.clear();

    renderer.setTheme({ ...TEST_THEME, cursor: rgba(1, 1, 1) });
    renderer.draw(frame);
    expect(fonts.measured).toBe(1);
    expect(backend.last.repaint).toBeUndefined();
    expect(rect(backend.planes!.overlayRect, 0)!.color).toBe(rgba(1, 1, 1));

    renderer.setTheme({ ...TEST_THEME, font: { family: "Test Mono", sizePx: 28 } });
    expect(renderer.metrics.cellWidth).toBe(16);
    expect(fonts.measured).toBe(2);
  });

  it("builds everything again when the atlas empties itself part-way through a frame", () => {
    const { renderer, backend, fonts } = setup();
    const first = makeFrame(80, 24);
    putText(first, 0, 0, "abc");
    drawn(renderer, first);
    const atlas = [...backend.atlases.values()][0]!;

    // Make every new glyph a quarter of a page, so thirty-six fill all the pages.
    fonts.rasterisers[0]!.inflate = ATLAS_PAGE_SIZE / 4;
    const filling = makeFrame(80, 24);
    putText(filling, 0, 0, "abc");
    for (let i = 0; i < 30; i++) putText(filling, i, 1, String.fromCodePoint(0x4e00 + i * 3));
    drawn(renderer, filling);
    expect(atlas.generation).toBe(0);

    // Twenty more do not fit beside those thirty, but do fit an emptied atlas.
    const frame = makeFrame(80, 24);
    putText(frame, 0, 0, "abc");
    for (let i = 0; i < 20; i++) putText(frame, i, 1, String.fromCodePoint(0x5e00 + i * 3));
    backend.clear();
    renderer.draw(frame);

    expect(atlas.generation).toBe(1);
    expect(backend.presents.length).toBe(1);
    expect(backend.last.repaint).toBeUndefined();
    // The first row was built before the atlas emptied. It was built again afterwards,
    // so its records point at where those glyphs are stored now.
    const a = glyph(backend.planes!.glyph, 0)!;
    const slot = atlas.glyph(0x61, false, false, 1);
    expect(atlas.generation).toBe(1);
    expect([a.u, a.v, a.page]).toEqual([atlas.u[slot], atlas.v[slot], atlas.page[slot]]);
  });

  it("stops drawing once disposed", () => {
    const { renderer, backend } = setup();
    renderer.dispose();
    renderer.dispose();
    renderer.draw(makeFrame(80, 24));
    expect(backend.disposed).toBe(true);
    expect(backend.presents).toEqual([]);
  });
});
