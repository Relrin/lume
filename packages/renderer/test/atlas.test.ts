import { describe, expect, it } from "vitest";

import { EMPTY_SLOT, GlyphAtlas } from "../src/atlas/atlas.ts";
import { classify } from "../src/atlas/classify.ts";
import { ShelfPacker } from "../src/atlas/packer.ts";
import { ATLAS_MAX_PAGES, ATLAS_PAGE_SIZE } from "../src/constants.ts";
import { UnderlineStyle } from "../src/frame.ts";
import { cellMetrics } from "../src/metrics.ts";
import { FakeRasteriser, RecordingSink } from "../src/testing/index.ts";

const metrics = cellMetrics(
  { advance: 8, ascent: 10.4, descent: 2.9 },
  { family: "x", sizePx: 14 },
  0,
  1,
);

function newAtlas() {
  const sink = new RecordingSink();
  const rasteriser = new FakeRasteriser(metrics);
  return { sink, rasteriser, atlas: new GlyphAtlas(sink, rasteriser, metrics) };
}

describe("the shelf packer", () => {
  it("places rectangles side by side with the gap between them", () => {
    const packer = new ShelfPacker(64, 1);
    expect(packer.place(10, 10)).toEqual({ x: 0, y: 0 });
    expect(packer.place(10, 10)).toEqual({ x: 11, y: 0 });
    expect(packer.place(10, 9)).toEqual({ x: 22, y: 0 });
  });

  it("opens a new shelf when the row is full or the rectangle is taller", () => {
    const packer = new ShelfPacker(32, 1);
    expect(packer.place(20, 8)).toEqual({ x: 0, y: 0 });
    expect(packer.place(20, 8)).toEqual({ x: 0, y: 9 });
    expect(packer.place(4, 12)).toEqual({ x: 0, y: 18 });
  });

  it("does not waste a tall shelf on a much shorter rectangle", () => {
    const packer = new ShelfPacker(64, 0);
    packer.place(10, 20);
    expect(packer.place(10, 4)).toEqual({ x: 0, y: 20 });
    expect(packer.place(10, 18)).toEqual({ x: 10, y: 0 });
  });

  it("refuses what does not fit and starts again after a clear", () => {
    const packer = new ShelfPacker(16, 0);
    expect(packer.place(17, 1)).toBeUndefined();
    expect(packer.place(16, 16)).toEqual({ x: 0, y: 0 });
    expect(packer.place(1, 1)).toBeUndefined();
    packer.clear();
    expect(packer.place(1, 1)).toEqual({ x: 0, y: 0 });
  });

  it("never places two rectangles over each other", () => {
    const packer = new ShelfPacker(128, 1);
    const placed: { x: number; y: number; w: number; h: number }[] = [];
    for (let i = 0; i < 400; i++) {
      const w = 3 + ((i * 7) % 17);
      const h = 5 + ((i * 5) % 13);
      const at = packer.place(w, h);
      if (at) placed.push({ ...at, w, h });
    }
    expect(placed.length).toBeGreaterThan(50);
    for (const a of placed) {
      expect(a.x + a.w).toBeLessThanOrEqual(128);
      expect(a.y + a.h).toBeLessThanOrEqual(128);
    }
    for (let i = 0; i < placed.length; i++) {
      for (let j = i + 1; j < placed.length; j++) {
        const a = placed[i]!;
        const b = placed[j]!;
        const apart = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
        expect(apart, `${i} and ${j} overlap`).toBe(true);
      }
    }
  });
});

describe("classifying a drawn glyph", () => {
  /** A 4 by 3 image with the given pixels set, everything else transparent. */
  function image(pixels: Record<string, [number, number, number, number]>): Uint8ClampedArray {
    const data = new Uint8ClampedArray(4 * 3 * 4);
    for (const [at, value] of Object.entries(pixels)) {
      const [x, y] = at.split(",").map(Number) as [number, number];
      data.set(value, (y * 4 + x) * 4);
    }
    return data;
  }

  it("reports no ink for an empty image", () => {
    expect(classify(image({}), 4, 3).width).toBe(0);
  });

  it("trims to the inked box and keeps coverage for white ink", () => {
    const ink = classify(image({ "1,1": [255, 255, 255, 200], "2,2": [255, 255, 255, 40] }), 4, 3);
    expect(ink).toMatchObject({ x: 1, y: 1, width: 2, height: 2, kind: "mask" });
    expect([...ink.pixels]).toEqual([200, 0, 0, 40]);
  });

  it("treats ink that is not white as colour, including black and grey", () => {
    for (const rgb of [
      [255, 0, 0],
      [0, 0, 0],
      [128, 128, 128],
    ] as const) {
      const ink = classify(image({ "0,0": [...rgb, 255] }), 4, 3);
      expect(ink.kind, rgb.join()).toBe("color");
      expect([...ink.pixels]).toEqual([...rgb, 255]);
    }
  });

  it("does not judge colour by pixels too faint to carry any", () => {
    const ink = classify(image({ "0,0": [255, 255, 255, 255], "1,0": [0, 0, 0, 3] }), 4, 3);
    expect(ink.kind).toBe("mask");
    expect(ink.width).toBe(2);
  });
});

describe("the glyph atlas", () => {
  it("draws a glyph once and returns the same slot afterwards", () => {
    const { atlas, rasteriser, sink } = newAtlas();
    const slot = atlas.glyph(0x41, false, false, 1);
    expect(slot).not.toBe(EMPTY_SLOT);
    expect(atlas.glyph(0x41, false, false, 1)).toBe(slot);
    expect(rasteriser.calls).toEqual(["A x1"]);
    expect(sink.uploads.length).toBe(1);
    expect(sink.pages.mask).toBe(1);
    expect([atlas.offsetX[slot], atlas.offsetY[slot]]).toEqual([1, 2]);
    expect([atlas.width[slot], atlas.height[slot]]).toEqual([6, metrics.cellHeight - 4]);
  });

  it("keeps bold, italic and wide variants apart", () => {
    const { atlas } = newAtlas();
    const slots = new Set([
      atlas.glyph(0x41, false, false, 1),
      atlas.glyph(0x41, true, false, 1),
      atlas.glyph(0x41, false, true, 1),
      atlas.glyph(0x41, true, true, 1),
      atlas.glyph(0x41, false, false, 2),
    ]);
    expect(slots.size).toBe(5);
  });

  it("gives a glyph without ink the empty slot, and remembers that too", () => {
    const { atlas, rasteriser, sink } = newAtlas();
    expect(atlas.glyph(0x20, false, false, 1)).toBe(EMPTY_SLOT);
    expect(atlas.glyph(0x20, false, false, 1)).toBe(EMPTY_SLOT);
    expect(rasteriser.calls.length).toBe(1);
    expect(sink.uploads.length).toBe(0);
  });

  it("stores colour glyphs in their own kind of page", () => {
    const { atlas, sink } = newAtlas();
    const slot = atlas.glyph(0x1f600, false, false, 2);
    expect(atlas.color[slot]).toBe(1);
    expect(sink.pages).toEqual({ mask: 0, color: 1 });
    expect(sink.uploads[0]!.kind).toBe("color");
  });

  it("looks clusters up by their text", () => {
    const { atlas, rasteriser } = newAtlas();
    const slot = atlas.cluster("é", false, false, 1);
    expect(atlas.cluster("é", false, false, 1)).toBe(slot);
    expect(atlas.cluster("é", true, false, 1)).not.toBe(slot);
    expect(rasteriser.calls.length).toBe(2);
  });

  it("draws box-drawing characters itself, the same for every weight", () => {
    const { atlas, rasteriser, sink } = newAtlas();
    const slot = atlas.glyph(0x2500, false, false, 1);
    expect(atlas.glyph(0x2500, true, true, 1)).toBe(slot);
    expect(rasteriser.calls).toEqual([]);
    expect([atlas.width[slot], atlas.height[slot]]).toEqual([
      metrics.cellWidth,
      metrics.cellHeight,
    ]);
    expect(sink.uploads[0]!.pixels.length).toBe(metrics.cellWidth * metrics.cellHeight);
  });

  it("keeps one sprite per underline style, one cell wide", () => {
    const { atlas } = newAtlas();
    const single = atlas.underline(UnderlineStyle.Single);
    const curly = atlas.underline(UnderlineStyle.Curly);
    expect(single).not.toBe(curly);
    expect(atlas.underline(UnderlineStyle.Single)).toBe(single);
    expect(atlas.width[single]).toBe(metrics.cellWidth);
    expect(atlas.height[single]).toBe(metrics.lineThickness);
    expect(atlas.height[curly]).toBeGreaterThan(metrics.lineThickness);
    expect(atlas.underline(UnderlineStyle.None)).toBe(EMPTY_SLOT);
  });

  it("adds pages as it fills, up to the limit, then empties itself", () => {
    const { atlas, rasteriser, sink } = newAtlas();
    // Glyphs a quarter of a page on a side: nine fit a page with the gap between them.
    rasteriser.inflate = ATLAS_PAGE_SIZE / 4;
    const perPage = 9;
    for (let i = 0; i < perPage * ATLAS_MAX_PAGES; i++) atlas.glyph(0x4e00 + i, false, false, 1);
    expect(atlas.pages("mask")).toBe(ATLAS_MAX_PAGES);
    expect(sink.pages.mask).toBe(ATLAS_MAX_PAGES);
    expect(atlas.generation).toBe(0);
    const first = atlas.glyph(0x4e00, false, false, 1);

    const overflow = atlas.glyph(0x9000, false, false, 1);
    expect(atlas.generation).toBe(1);
    expect(atlas.size).toBe(1);
    expect(overflow).toBe(1);
    // The earlier glyph is gone and is drawn again when asked for.
    const calls = rasteriser.calls.length;
    expect(atlas.glyph(0x4e00, false, false, 1)).not.toBe(first);
    expect(rasteriser.calls.length).toBe(calls + 1);
  });

  it("gives a glyph larger than a page the empty slot", () => {
    const { atlas, rasteriser } = newAtlas();
    rasteriser.inflate = ATLAS_PAGE_SIZE;
    expect(atlas.glyph(0x41, false, false, 1)).toBe(EMPTY_SLOT);
  });

  it("grows its tables past their first size", () => {
    const { atlas } = newAtlas();
    let last = 0;
    for (let i = 0; i < 3000; i++) last = atlas.glyph(0x4e00 + i, false, false, 1);
    expect(last).toBe(3000);
    expect(atlas.width[last]).toBe(6);
    expect(atlas.generation).toBe(0);
  });
});
