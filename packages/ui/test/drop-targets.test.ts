import { describe, expect, it } from "vitest";

import { panelDropArea } from "../src/drag/panels.ts";
import { dropTarget } from "../src/drag/targets.ts";
import { dragFloat, keepInside } from "../src/layout/floats.ts";
import type { PaneRect } from "../src/layout/geometry.ts";

/** Two cards side by side with a gap of eight between them, and a third below both. */
const PANES: readonly PaneRect[] = [
  { pane: "p1", rect: { x: 0, y: 0, width: 400, height: 300 } },
  { pane: "p2", rect: { x: 408, y: 0, width: 400, height: 300 } },
  { pane: "p3", rect: { x: 0, y: 308, width: 808, height: 200 } },
];

describe("where a dragged card would go", () => {
  it("goes beside a card when it is held near one of the card's edges", () => {
    expect(dropTarget({ x: 420, y: 150 }, PANES, "p1")).toEqual({
      kind: "beside",
      pane: "p2",
      edge: "left",
      rect: { x: 408, y: 0, width: 200, height: 300 },
    });
    expect(dropTarget({ x: 800, y: 150 }, PANES, "p1")).toMatchObject({
      pane: "p2",
      edge: "right",
      rect: { x: 608, y: 0, width: 200, height: 300 },
    });
    expect(dropTarget({ x: 600, y: 10 }, PANES, "p1")).toMatchObject({
      pane: "p2",
      edge: "up",
      rect: { x: 408, y: 0, width: 400, height: 150 },
    });
    expect(dropTarget({ x: 600, y: 290 }, PANES, "p1")).toMatchObject({
      pane: "p2",
      edge: "down",
      rect: { x: 408, y: 150, width: 400, height: 150 },
    });
  });

  it("changes places with a card when it is held over the card's middle", () => {
    expect(dropTarget({ x: 608, y: 150 }, PANES, "p1")).toEqual({
      kind: "swap",
      pane: "p2",
      rect: { x: 408, y: 0, width: 400, height: 300 },
    });
  });

  it("takes the nearer edge in a corner", () => {
    // Twelve across and thirty down the wide card: nearer its left edge, as shares.
    expect(dropTarget({ x: 12, y: 338 }, PANES, "p1")).toMatchObject({ pane: "p3", edge: "left" });
    // Two hundred across and ten down: nearer its top.
    expect(dropTarget({ x: 200, y: 318 }, PANES, "p1")).toMatchObject({ pane: "p3", edge: "up" });
  });

  it("goes nowhere over its own card, a gap, or nothing at all", () => {
    expect(dropTarget({ x: 100, y: 100 }, PANES, "p1")).toBeUndefined();
    expect(dropTarget({ x: 404, y: 100 }, PANES, "p1")).toBeUndefined();
    expect(dropTarget({ x: 2000, y: 100 }, PANES, "p1")).toBeUndefined();
    expect(dropTarget({ x: 100, y: 100 }, [], "p1")).toBeUndefined();
  });

  it("takes the share of a card that counts as its edge from whoever asks", () => {
    expect(dropTarget({ x: 500, y: 150 }, PANES, "p1", 0.1)).toMatchObject({ kind: "swap" });
    expect(dropTarget({ x: 500, y: 150 }, PANES, "p1", 0.3)).toMatchObject({ edge: "left" });
  });
});

describe("where a dragged panel would be docked", () => {
  const body = { x: 0, y: 44, width: 900, height: 600 };
  const all = ["left", "right", "bottom"] as const;

  it("is docked at the side it is held near", () => {
    expect(panelDropArea({ x: 100, y: 300 }, body, all)).toBe("left");
    expect(panelDropArea({ x: 800, y: 300 }, body, all)).toBe("right");
    expect(panelDropArea({ x: 450, y: 600 }, body, all)).toBe("bottom");
    expect(panelDropArea({ x: 450, y: 200 }, body, all)).toBeNull();
  });

  it("gives the lower corners to the columns", () => {
    expect(panelDropArea({ x: 50, y: 630 }, body, all)).toBe("left");
    expect(panelDropArea({ x: 850, y: 630 }, body, all)).toBe("right");
  });

  it("offers only the sides the panel may go to", () => {
    const sides = ["left", "right"] as const;
    expect(panelDropArea({ x: 450, y: 600 }, body, sides)).toBeNull();
    expect(panelDropArea({ x: 100, y: 600 }, body, sides)).toBe("left");
  });

  it("is docked nowhere outside the window's body", () => {
    expect(panelDropArea({ x: 100, y: 10 }, body, all)).toBeNull();
    expect(panelDropArea({ x: -5, y: 300 }, body, all)).toBeNull();
    expect(panelDropArea({ x: 100, y: 300 }, { ...body, width: 0 }, all)).toBeNull();
  });
});

describe("moving and resizing a floating pane", () => {
  const box = { x: 8, y: 0, width: 800, height: 600 };
  const start = { x: 100, y: 100, width: 400, height: 300 };
  const min = { width: 240, height: 120 };

  it("moves with the pointer and keeps its size", () => {
    expect(dragFloat(start, "move", 50, -20, box, min)).toEqual({ ...start, x: 150, y: 80 });
  });

  it("stops at the edges of the space it floats over", () => {
    expect(dragFloat(start, "move", -500, -500, box, min)).toEqual({ ...start, x: 8, y: 0 });
    expect(dragFloat(start, "move", 5000, 5000, box, min)).toEqual({ ...start, x: 408, y: 300 });
  });

  it("moves one edge and leaves the others where they are", () => {
    expect(dragFloat(start, "e", 60, 999, box, min)).toEqual({ ...start, width: 460 });
    expect(dragFloat(start, "w", 60, 999, box, min)).toEqual({ ...start, x: 160, width: 340 });
    expect(dragFloat(start, "s", 999, 40, box, min)).toEqual({ ...start, height: 340 });
    expect(dragFloat(start, "se", 10, 20, box, min)).toEqual({ ...start, width: 410, height: 320 });
    expect(dragFloat(start, "sw", -10, 20, box, min)).toEqual({
      x: 90,
      y: 100,
      width: 410,
      height: 320,
    });
  });

  it("is never made smaller than a pane may be, nor larger than its space", () => {
    expect(dragFloat(start, "e", -1000, 0, box, min).width).toBe(240);
    expect(dragFloat(start, "w", 1000, 0, box, min)).toEqual({ ...start, x: 260, width: 240 });
    expect(dragFloat(start, "s", 0, -1000, box, min).height).toBe(120);
    expect(dragFloat(start, "e", 5000, 0, box, min).width).toBe(708);
    expect(dragFloat(start, "w", -5000, 0, box, min)).toEqual({ ...start, x: 8, width: 492 });
    expect(dragFloat(start, "s", 0, 5000, box, min).height).toBe(500);
  });

  it("allows what a space smaller than the smallest pane has", () => {
    const tight = { x: 0, y: 0, width: 200, height: 100 };
    const pane = { x: 0, y: 0, width: 200, height: 100 };
    expect(dragFloat(pane, "e", -500, 0, tight, min).width).toBe(200);
    expect(dragFloat(pane, "s", 0, -500, tight, min).height).toBe(100);
  });

  it("is brought inside a space it has come to lie outside of", () => {
    expect(keepInside({ x: 700, y: 500, width: 400, height: 300 }, box)).toEqual({
      x: 408,
      y: 300,
      width: 400,
      height: 300,
    });
    expect(keepInside({ x: 0, y: 0, width: 2000, height: 50 }, box)).toEqual({
      x: 8,
      y: 0,
      width: 800,
      height: 50,
    });
  });
});
