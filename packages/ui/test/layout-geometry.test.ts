import { describe, expect, it } from "vitest";

import {
  computeRects,
  dragRatios,
  equalRatios,
  pxToUnitRect,
  unitRectToPx,
  type Rect,
  type Rects,
  type ShapeNode,
} from "../src/layout/geometry.ts";
import { SHARE_MIN, UNITS } from "../src/layout/types.ts";
import { DIVIDER_HIT_MIN_PX } from "../src/tuning.ts";
import { pane, split } from "./layout-support.ts";

const BOX: Rect = { x: 8, y: 44, width: 1000, height: 600 };
const GAP = 8;

function rectOf(rects: Rects, id: string): Rect {
  const found = rects.panes.find((placed) => placed.pane === id);
  if (!found) throw new Error(`no pane ${id}`);
  return found.rect;
}

function sum(values: readonly number[]): number {
  return values.reduce((total, value) => total + value, 0);
}

function overlap(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/** Every pane inside the box, in whole pixels, and no two overlapping. */
function expectSound(rects: Rects, box: Rect): void {
  for (const { rect } of rects.panes) {
    for (const value of [rect.x, rect.y, rect.width, rect.height]) {
      expect(Number.isInteger(value)).toBe(true);
    }
    expect(rect.x).toBeGreaterThanOrEqual(box.x);
    expect(rect.y).toBeGreaterThanOrEqual(box.y);
    expect(rect.x + rect.width).toBeLessThanOrEqual(box.x + box.width);
    expect(rect.y + rect.height).toBeLessThanOrEqual(box.y + box.height);
  }
  rects.panes.forEach((a, index) => {
    for (const b of rects.panes.slice(index + 1)) expect(overlap(a.rect, b.rect)).toBe(false);
  });
}

describe("laying a tab out", () => {
  it("gives one pane the whole box", () => {
    const rects = computeRects(pane("p1"), BOX, GAP);
    expect(rects.panes).toEqual([{ pane: "p1", rect: BOX }]);
    expect(rects.dividers).toEqual([]);
  });

  it("puts two panes side by side with the gap between them", () => {
    const rects = computeRects(
      split("s1", "row", [500_000, 500_000], [pane("p1"), pane("p2")]),
      BOX,
      GAP,
    );
    expect(rectOf(rects, "p1")).toEqual({ x: 8, y: 44, width: 496, height: 600 });
    expect(rectOf(rects, "p2")).toEqual({ x: 512, y: 44, width: 496, height: 600 });
    expect(rects.dividers).toEqual([
      {
        split: "s1",
        index: 0,
        axis: "row",
        rect: { x: 504, y: 44, width: 8, height: 600 },
        gap: { x: 504, y: 44, width: 8, height: 600 },
        beforePx: 496,
        afterPx: 496,
        extentPx: 1000,
      },
    ]);
    expectSound(rects, BOX);
  });

  it("puts one pane above the other in a column", () => {
    const rects = computeRects(
      split("s1", "column", [250_000, 750_000], [pane("p1"), pane("p2")]),
      BOX,
      GAP,
    );
    expect(rectOf(rects, "p1")).toEqual({ x: 8, y: 44, width: 1000, height: 148 });
    expect(rectOf(rects, "p2")).toEqual({ x: 8, y: 200, width: 1000, height: 444 });
    expect(rects.dividers[0]).toMatchObject({ axis: "column", beforePx: 148, afterPx: 444 });
  });

  it("gives the last of three what rounding left over", () => {
    const third = Math.floor(UNITS / 3);
    const ratios = [third, third, UNITS - 2 * third];
    const tree = split("s1", "row", ratios, [pane("p1"), pane("p2"), pane("p3")]);
    const box = { x: 0, y: 0, width: 1001, height: 300 };
    const rects = computeRects(tree, box, 7);
    const widths = rects.panes.map((placed) => placed.rect.width);
    expect(sum(widths) + 2 * 7).toBe(1001);
    expect(rects.panes.map((placed) => placed.rect.x)).toEqual([
      0,
      widths[0]! + 7,
      widths[0]! + widths[1]! + 14,
    ]);
    expect(rects.dividers.map((divider) => divider.index)).toEqual([0, 1]);
    expectSound(rects, box);
  });

  it("lays nested splits out inside their share", () => {
    const tree = split(
      "s1",
      "row",
      [600_000, 400_000],
      [
        pane("p1"),
        split(
          "s2",
          "column",
          [500_000, 500_000],
          [pane("p2"), split("s3", "row", [300_000, 700_000], [pane("p3"), pane("p4")])],
        ),
      ],
    );
    const rects = computeRects(tree, BOX, GAP);
    expect(rects.panes.map((placed) => placed.pane)).toEqual(["p1", "p2", "p3", "p4"]);
    expect(rectOf(rects, "p1")).toEqual({ x: 8, y: 44, width: 595, height: 600 });
    expect(rectOf(rects, "p2")).toEqual({ x: 611, y: 44, width: 397, height: 296 });
    expect(rectOf(rects, "p3")).toEqual({ x: 611, y: 348, width: 116, height: 296 });
    expect(rectOf(rects, "p4")).toEqual({ x: 735, y: 348, width: 273, height: 296 });
    expect(rects.dividers.map((divider) => divider.split)).toEqual(["s1", "s2", "s3"]);
    expect(rects.dividers[2]).toMatchObject({ extentPx: 397, beforePx: 116, afterPx: 273 });
    expectSound(rects, BOX);
  });

  it("takes a box that is not in whole pixels to whole pixels", () => {
    const box = { x: 0.5, y: 0.25, width: 800.4, height: 500.6 };
    const tree = split("s1", "row", [333_333, 666_667], [pane("p1"), pane("p2")]);
    const rects = computeRects(tree, box, GAP);
    expectSound(rects, { x: 1, y: 0, width: 800, height: 501 });
    expect(rectOf(rects, "p1").width + rectOf(rects, "p2").width + GAP).toBe(800);
  });

  it("lets the zoomed pane fill the box alone", () => {
    const tree = split("s1", "row", [500_000, 500_000], [pane("p1"), pane("p2")]);
    const rects = computeRects(tree, BOX, GAP, { zoomed: "p2" });
    expect(rects).toEqual({ panes: [{ pane: "p2", rect: BOX }], dividers: [] });
    // A zoom on a pane the tree does not hold is no zoom.
    expect(computeRects(tree, BOX, GAP, { zoomed: "p9" }).panes).toHaveLength(2);
  });

  it("takes ratios from elsewhere when it is given them", () => {
    const shape: ShapeNode = {
      type: "split",
      id: "s1",
      axis: "row",
      children: [
        { type: "pane", id: "p1" },
        { type: "pane", id: "p2" },
      ],
    };
    const looked = computeRects(shape, BOX, GAP, {
      ratios: (id) => (id === "s1" ? [250_000, 750_000] : undefined),
    });
    expect(rectOf(looked, "p1").width).toBe(248);
    expect(rectOf(looked, "p2").width).toBe(744);
    // With none anywhere the children share alike.
    const alike = computeRects(shape, BOX, GAP);
    expect(rectOf(alike, "p1").width).toBe(496);
    // A drag tries its own over the tree's.
    const tree = split("s1", "row", [500_000, 500_000], [pane("p1"), pane("p2")]);
    const tried = computeRects(tree, BOX, GAP, { ratios: () => [100_000, 900_000] });
    expect(rectOf(tried, "p1").width).toBe(99);
  });

  it("widens a narrow gap evenly for the pointer and leaves the gap itself", () => {
    const tree = split("s1", "row", [500_000, 500_000], [pane("p1"), pane("p2")]);
    const rects = computeRects(tree, { x: 0, y: 0, width: 402, height: 100 }, 2);
    const divider = rects.dividers[0]!;
    expect(divider.gap).toEqual({ x: 200, y: 0, width: 2, height: 100 });
    expect(divider.rect).toEqual({ x: 198, y: 0, width: DIVIDER_HIT_MIN_PX, height: 100 });
    const none = computeRects(tree, { x: 0, y: 0, width: 400, height: 100 }, 0);
    expect(none.dividers[0]!.rect.width).toBe(DIVIDER_HIT_MIN_PX);
    expect(rectOf(none, "p1").width + rectOf(none, "p2").width).toBe(400);
  });

  it("never gives a pane less than nothing in a box too small for its gaps", () => {
    const tree = split("s1", "row", equalRatios(4), [
      pane("p1"),
      pane("p2"),
      pane("p3"),
      pane("p4"),
    ]);
    const rects = computeRects(tree, { x: 0, y: 0, width: 10, height: 10 }, 8);
    for (const { rect } of rects.panes) expect(rect.width).toBeGreaterThanOrEqual(0);
  });
});

describe("turning a dragged divider into ratios", () => {
  const half = [500_000, 500_000];

  it("moves the share between the two children either side", () => {
    // 1000 px less one gap of 8 is 992 px to share: 99.2 px is a tenth.
    expect(dragRatios(half, 0, 99.2, 1000, 8, 0)).toEqual([600_000, 400_000]);
    expect(dragRatios(half, 0, -99.2, 1000, 8, 0)).toEqual([400_000, 600_000]);
  });

  it("leaves every other child's share as it was", () => {
    const ratios = [200_000, 300_000, 400_000, 100_000];
    const dragged = dragRatios(ratios, 1, 50, 1024, 8, 0);
    expect(dragged[0]).toBe(200_000);
    expect(dragged[3]).toBe(100_000);
    expect(dragged[1]! + dragged[2]!).toBe(700_000);
    expect(dragged[1]).toBe(350_000);
    expect(sum(dragged)).toBe(UNITS);
  });

  it("stops at the smallest size a pane may have", () => {
    // 120 px of 992 is 120 968 units, rounded up.
    const least = Math.ceil((120 * UNITS) / 992);
    expect(dragRatios(half, 0, -5000, 1000, 8, 120)).toEqual([least, UNITS - least]);
    expect(dragRatios(half, 0, 5000, 1000, 8, 120)).toEqual([UNITS - least, least]);
  });

  it("stops at the smallest share when the pixels allow less", () => {
    expect(dragRatios(half, 0, -5000, 4000, 8, 10)).toEqual([SHARE_MIN, UNITS - SHARE_MIN]);
  });

  it("keeps the smallest share alone where both cannot have the smallest size", () => {
    const ratios = [100_000, 100_000, 800_000];
    // The two share 198 px: not room for 120 px each.
    const dragged = dragRatios(ratios, 0, 1000, 1000, 5, 120);
    expect(dragged).toEqual([200_000 - SHARE_MIN, SHARE_MIN, 800_000]);
  });

  it("changes nothing where there is no room to move, or no such divider", () => {
    const tight = [SHARE_MIN, SHARE_MIN, UNITS - 2 * SHARE_MIN];
    expect(dragRatios(tight, 0, 300, 1000, 8, 0)).toEqual(tight);
    expect(dragRatios(half, 1, 300, 1000, 8, 0)).toEqual(half);
    expect(dragRatios(half, 0, 300, 4, 8, 0)).toEqual(half);
  });

  it("always adds up to a whole in whole units", () => {
    const ratios = [333_333, 333_333, 333_334];
    for (const delta of [-700, -33.3, -1, 0, 0.4, 12.7, 250, 9999]) {
      for (const index of [0, 1]) {
        const dragged = dragRatios(ratios, index, delta, 977, 6, 60);
        expect(sum(dragged)).toBe(UNITS);
        for (const ratio of dragged) {
          expect(Number.isInteger(ratio)).toBe(true);
          expect(ratio).toBeGreaterThanOrEqual(SHARE_MIN);
        }
      }
    }
  });
});

describe("equal shares", () => {
  it("add up to a whole for any number of children", () => {
    for (let count = 1; count <= 20; count++) {
      const ratios = equalRatios(count);
      expect(ratios).toHaveLength(count);
      expect(sum(ratios)).toBe(UNITS);
      expect(Math.max(...ratios) - Math.min(...ratios)).toBeLessThan(count);
    }
    expect(equalRatios(3)).toEqual([333_333, 333_333, 333_334]);
    expect(equalRatios(0)).toEqual([]);
  });
});

describe("a floating pane's rectangle", () => {
  it("is laid on the box in pixels and read back in units", () => {
    const units = { x: 200_000, y: 200_000, w: 600_000, h: 600_000 };
    const px = unitRectToPx(units, BOX);
    expect(px).toEqual({ x: 208, y: 164, width: 600, height: 360 });
    expect(pxToUnitRect(px, BOX)).toEqual(units);
  });

  it("stays inside the whole when read back from outside the box", () => {
    const far = pxToUnitRect({ x: -50, y: 700, width: 2000, height: 10 }, BOX);
    expect(far).toEqual({ x: 0, y: UNITS, w: UNITS, h: 0 });
    expect(pxToUnitRect(BOX, { ...BOX, width: 0 }).w).toBe(0);
  });
});
