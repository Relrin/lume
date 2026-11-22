import { describe, expect, it } from "vitest";

import {
  AWAY_PX,
  away,
  EDGE_SPEED_MAX_PX,
  EDGE_ZONE_PX,
  edgeSpeed,
  fades,
  tabMatches,
  wheelShift,
  type Span,
} from "../src/shell/tab-scroll.ts";

describe("which ends of a row of tabs fade", () => {
  it("is neither while everything fits", () => {
    expect(fades({ scroll: 0, view: 800, full: 800 })).toEqual({
      start: false,
      end: false,
      over: false,
    });
    expect(fades({ scroll: 0, view: 800, full: 640 })).toEqual({
      start: false,
      end: false,
      over: false,
    });
  });

  it("is the end that hides tabs, and only that one", () => {
    expect(fades({ scroll: 0, view: 800, full: 1400 })).toEqual({
      start: false,
      end: true,
      over: true,
    });
    expect(fades({ scroll: 300, view: 800, full: 1400 })).toEqual({
      start: true,
      end: true,
      over: true,
    });
    expect(fades({ scroll: 600, view: 800, full: 1400 })).toEqual({
      start: true,
      end: false,
      over: true,
    });
  });

  it("is not moved by a pixel of rounding", () => {
    expect(fades({ scroll: 0.6, view: 800, full: 1400 }).start).toBe(false);
    expect(fades({ scroll: 599.4, view: 800, full: 1400 }).end).toBe(false);
    expect(fades({ scroll: 0, view: 800, full: 800.8 })).toEqual({
      start: false,
      end: false,
      over: false,
    });
  });
});

describe("what a turn of the wheel moves the row by", () => {
  it("is the turn, sideways, for a wheel that turns up and down", () => {
    expect(wheelShift(0, 120, 0, 800)).toBe(120);
    expect(wheelShift(0, -120, 0, 800)).toBe(-120);
    expect(wheelShift(10, 120, 0, 800)).toBe(120);
  });

  it("is nothing of the row's doing for a wheel that already goes sideways", () => {
    expect(wheelShift(80, 0, 0, 800)).toBeNull();
    expect(wheelShift(80, 12, 0, 800)).toBeNull();
    expect(wheelShift(-40, 40, 0, 800)).toBeNull();
    expect(wheelShift(0, 0, 0, 800)).toBeNull();
  });

  it("is in the wheel's own unit: pixels, lines or pages", () => {
    expect(wheelShift(0, 3, 1, 800)).toBe(48);
    expect(wheelShift(0, -1, 2, 800)).toBe(-800);
  });
});

describe("how fast the row moves under a tab held at its end", () => {
  const start = 100;
  const end = 900;

  it("is nothing in the middle of the row", () => {
    expect(edgeSpeed(500, start, end)).toBe(0);
    expect(edgeSpeed(start + EDGE_ZONE_PX, start, end)).toBe(0);
    expect(edgeSpeed(end - EDGE_ZONE_PX, start, end)).toBe(0);
  });

  it("is towards the end the pointer is near, faster the nearer it is", () => {
    const slow = edgeSpeed(end - EDGE_ZONE_PX + 4, start, end);
    const fast = edgeSpeed(end - 2, start, end);
    expect(slow).toBeGreaterThan(0);
    expect(fast).toBeGreaterThan(slow);
    expect(edgeSpeed(start + 2, start, end)).toBe(-fast);
    expect(edgeSpeed(start + EDGE_ZONE_PX - 4, start, end)).toBe(-slow);
  });

  it("is never more than its most, however far past the end the pointer goes", () => {
    expect(edgeSpeed(end + 400, start, end)).toBe(EDGE_SPEED_MAX_PX);
    expect(edgeSpeed(start - 400, start, end)).toBe(-EDGE_SPEED_MAX_PX);
  });

  it("is nothing in a row too short to have a middle", () => {
    expect(edgeSpeed(10, 0, EDGE_ZONE_PX)).toBe(0);
  });
});

describe("the tabs that are out of sight", () => {
  /** Tabs a hundred wide, the row showing 0 to 300. */
  const spans: Span[] = [-250, -150, -90, 0, 100, 200, 290, 380].map((start, index) => ({
    id: `t${index + 1}`,
    start,
    end: start + 100,
  }));

  it("are those of which no more than a sliver shows", () => {
    expect(away(spans, 0, 300)).toEqual(["t1", "t2", "t3", "t7", "t8"]);
  });

  it("are not those that are partly hidden but can still be told", () => {
    const half: Span[] = [
      { id: "a", start: -100 + AWAY_PX + 1, end: AWAY_PX + 1 },
      { id: "b", start: 300 - AWAY_PX - 1, end: 400 - AWAY_PX - 1 },
    ];
    expect(away(half, 0, 300)).toEqual([]);
  });

  it("are none while everything fits", () => {
    expect(away(spans.slice(3, 6), 0, 300)).toEqual([]);
  });
});

describe("finding a tab by what it is called", () => {
  it("holds for any tab while nothing was typed", () => {
    expect(tabMatches("cargo test", "")).toBe(true);
    expect(tabMatches("cargo test", "   ")).toBe(true);
  });

  it("looks anywhere in the name, whatever the letters' case", () => {
    expect(tabMatches("Fix tax rounding", "TAX")).toBe(true);
    expect(tabMatches("Fix tax rounding", " round ")).toBe(true);
    expect(tabMatches("Fix tax rounding", "deploy")).toBe(false);
  });
});
