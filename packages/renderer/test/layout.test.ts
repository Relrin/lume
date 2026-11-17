import { describe, expect, it } from "vitest";

import { LINE_RECTS_PER_CELL } from "../src/constants.ts";
import { collectDirtyRows, rowRuns } from "../src/dirty.ts";
import { DirtyState, NO_SELECTION } from "../src/frame.ts";
import {
  clearSlots,
  createPlanes,
  GLYPH,
  isLittleEndian,
  PLANE_ORDER,
  RECT,
  writeGlyph,
  writeRect,
} from "../src/layout.ts";
import { cellMetrics, fontKey, gridSize } from "../src/metrics.ts";
import { makeFrame, markDrawn, select, touchRow } from "../src/testing/index.ts";

describe("record layouts", () => {
  it("keep every offset and the record size a multiple of four", () => {
    for (const layout of [RECT, GLYPH]) {
      expect(layout.stride % 4).toBe(0);
      for (const field of layout.fields) expect(field.offset % 4, field.name).toBe(0);
    }
  });

  it("pack their fields end to end with nothing left over", () => {
    for (const layout of [RECT, GLYPH]) {
      let offset = 0;
      for (const field of layout.fields) {
        expect(field.offset, field.name).toBe(offset);
        offset += 4;
      }
      expect(layout.stride).toBe(offset);
    }
  });

  it("run on a little-endian machine", () => {
    expect(isLittleEndian()).toBe(true);
  });

  it("write a rectangle as position, size and colour bytes", () => {
    const planes = createPlanes(4);
    writeRect(planes.background, 2, -3, 300, 9, 18, 0x44332211);
    const view = new DataView(planes.background.bytes.buffer, 2 * RECT.stride, RECT.stride);
    expect(view.getInt16(0, true)).toBe(-3);
    expect(view.getInt16(2, true)).toBe(300);
    expect(view.getUint16(4, true)).toBe(9);
    expect(view.getUint16(6, true)).toBe(18);
    // Red, green, blue, alpha in that order in memory.
    expect([view.getUint8(8), view.getUint8(9), view.getUint8(10), view.getUint8(11)]).toEqual([
      0x11, 0x22, 0x33, 0x44,
    ]);
    // The neighbouring slots are untouched.
    expect(planes.background.bytes.subarray(0, 2 * RECT.stride).every((b) => b === 0)).toBe(true);
    expect(planes.background.bytes.subarray(3 * RECT.stride).every((b) => b === 0)).toBe(true);
  });

  it("write a glyph with its place in the atlas", () => {
    const planes = createPlanes(4);
    writeGlyph(planes.glyph, 1, 10, 20, 7, 14, 100, 200, 3, 1, 0xffaabbcc);
    const view = new DataView(planes.glyph.bytes.buffer, GLYPH.stride, GLYPH.stride);
    const words = Array.from({ length: 8 }, (_, i) => view.getUint16(i * 2, true));
    expect(words).toEqual([10, 20, 7, 14, 100, 200, 3, 1]);
    expect(view.getUint32(16, true)).toBe(0xffaabbcc);
  });

  it("clear a run of slots and only that run", () => {
    const planes = createPlanes(4);
    for (let slot = 0; slot < 4; slot++) writeRect(planes.background, slot, 1, 1, 1, 1, 0xffffffff);
    clearSlots(planes.background, 1, 2);
    const used = (slot: number) =>
      planes.background.bytes.subarray(slot * 12, slot * 12 + 12).some((b) => b !== 0);
    expect([0, 1, 2, 3].map(used)).toEqual([true, false, false, true]);
  });

  it("size each plane for the grid", () => {
    const planes = createPlanes(100);
    expect(planes.background.bytes.byteLength).toBe(100 * RECT.stride);
    expect(planes.glyph.bytes.byteLength).toBe(100 * GLYPH.stride);
    expect(planes.underline.bytes.byteLength).toBe(100 * GLYPH.stride);
    expect(planes.line.bytes.byteLength).toBe(100 * LINE_RECTS_PER_CELL * RECT.stride);
    expect(Object.keys(planes)).toEqual([...PLANE_ORDER]);
  });
});

describe("cell metrics", () => {
  const font = { family: "x", sizePx: 14 };

  it("round the cell to whole pixels and centre the text in the line", () => {
    const m = cellMetrics({ advance: 8.4, ascent: 13.2, descent: 3.4 }, font, 0, 1);
    expect(m.cellWidth).toBe(8);
    // 16.6 pixels of text at a line height of 1.2.
    expect(m.cellHeight).toBe(20);
    expect(m.baseline).toBe(Math.round((20 - 16.6) / 2 + 13.2));
    expect(m.lineThickness).toBe(1);
  });

  it("never make a cell or a line thinner than one pixel", () => {
    const m = cellMetrics({ advance: 0.2, ascent: 0.2, descent: 0.1 }, font, 0, 1);
    expect([m.cellWidth, m.cellHeight, m.lineThickness]).toEqual([1, 1, 1]);
  });

  it("keep the underline and the strikethrough inside the cell", () => {
    for (const lineHeight of [0.1, 0.5, 0.8, 1, 1.2, 2, 3]) {
      const m = cellMetrics({ advance: 9, ascent: 14, descent: 9 }, { ...font, lineHeight }, 0, 1);
      expect(m.cellHeight).toBeGreaterThanOrEqual(1);
      expect(m.underlineTop + m.lineThickness).toBeLessThanOrEqual(m.cellHeight);
      expect(m.strikethroughTop + m.lineThickness).toBeLessThanOrEqual(m.cellHeight);
      expect(m.strikethroughTop).toBeGreaterThanOrEqual(0);
      expect(m.strikethroughTop).toBeLessThan(m.baseline);
    }
  });

  it("scale the padding by the device pixel ratio", () => {
    expect(cellMetrics({ advance: 8, ascent: 12, descent: 4 }, font, 6, 1.5).padding).toBe(9);
  });

  it("fit whole cells inside the padding, and at least one", () => {
    const m = cellMetrics({ advance: 8, ascent: 12, descent: 4 }, font, 4, 1);
    expect(gridSize({ width: 8 * 80 + 8, height: m.cellHeight * 24 + 8 }, m)).toEqual({
      cols: 80,
      rows: 24,
    });
    expect(gridSize({ width: 8 * 80 + 7, height: m.cellHeight * 24 + 7 }, m)).toEqual({
      cols: 79,
      rows: 23,
    });
    expect(gridSize({ width: 0, height: 0 }, m)).toEqual({ cols: 1, rows: 1 });
  });

  it("give a different key whenever glyph pixels would differ", () => {
    const m = cellMetrics({ advance: 8, ascent: 12, descent: 4 }, font, 0, 1);
    const key = fontKey(font, 1, m);
    expect(fontKey(font, 1, m)).toBe(key);
    expect(fontKey(font, 2, m)).not.toBe(key);
    expect(fontKey({ ...font, sizePx: 15 }, 1, m)).not.toBe(key);
    expect(fontKey({ ...font, family: "y" }, 1, m)).not.toBe(key);
    expect(fontKey(font, 1, { ...m, cellHeight: m.cellHeight + 1 })).not.toBe(key);
  });
});

describe("dirty rows", () => {
  const state = (rows: number) => ({
    start: new Int32Array(rows).fill(NO_SELECTION),
    end: new Int32Array(rows),
    out: new Uint8Array(rows),
  });

  it("marks every row for a full frame or when forced", () => {
    const frame = makeFrame(4, 3);
    const s = state(3);
    expect(collectDirtyRows(frame, false, s.start, s.end, s.out)).toBe(3);
    markDrawn(frame);
    expect(collectDirtyRows(frame, false, s.start, s.end, s.out)).toBe(0);
    expect(collectDirtyRows(frame, true, s.start, s.end, s.out)).toBe(3);
  });

  it("marks the rows the frame flags", () => {
    const frame = makeFrame(4, 5);
    const s = state(5);
    collectDirtyRows(frame, false, s.start, s.end, s.out);
    markDrawn(frame);
    touchRow(frame, 1);
    touchRow(frame, 3);
    expect(frame.dirty).toBe(DirtyState.Partial);
    expect(collectDirtyRows(frame, false, s.start, s.end, s.out)).toBe(2);
    expect([...s.out]).toEqual([0, 1, 0, 1, 0]);
  });

  it("marks a row whose selection changed even if the frame does not flag it", () => {
    const frame = makeFrame(8, 4);
    const s = state(4);
    collectDirtyRows(frame, false, s.start, s.end, s.out);
    markDrawn(frame);

    select(frame, 2, 1, 5);
    expect(collectDirtyRows(frame, false, s.start, s.end, s.out)).toBe(1);
    expect([...s.out]).toEqual([0, 0, 1, 0]);
    // Unchanged since: nothing to do.
    expect(collectDirtyRows(frame, false, s.start, s.end, s.out)).toBe(0);
    select(frame, 2, 1, 6);
    expect(collectDirtyRows(frame, false, s.start, s.end, s.out)).toBe(1);
    frame.selectionStart[2] = NO_SELECTION;
    expect(collectDirtyRows(frame, false, s.start, s.end, s.out)).toBe(1);
  });

  it("groups marked rows into runs", () => {
    expect(rowRuns(Uint8Array.of(0, 0, 0), 3)).toEqual([]);
    expect(rowRuns(Uint8Array.of(1, 1, 1), 3)).toEqual([0, 3]);
    expect(rowRuns(Uint8Array.of(1, 0, 1, 1, 0, 0, 1), 7)).toEqual([0, 1, 2, 4, 6, 7]);
    expect(rowRuns(Uint8Array.of(0, 1, 1, 1), 3)).toEqual([1, 3]);
  });
});
