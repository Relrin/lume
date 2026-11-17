import { describe, expect, it } from "vitest";

import {
  bezier,
  blend,
  MOTION_STYLES,
  rgbOf,
  slideCurve,
  slideOffset,
  springCurve,
  springsFor,
  stepEnds,
  type Ends,
} from "../src/shell/tab-motion.ts";

describe("the curves tabs move along", () => {
  it("brings a spring to rest at its end, a little past it on the way", () => {
    const curve = springCurve([600, 0.72]);
    expect(curve.at(0)).toBe(0);
    expect(curve.at(1)).toBe(1);
    const samples = Array.from({ length: 101 }, (_, index) => curve.at(index / 100));
    expect(Math.max(...samples)).toBeGreaterThan(1);
    expect(curve.duration).toBeGreaterThan(100);
    expect(curve.duration).toBeLessThan(1200);
    expect(curve.easing.startsWith("linear(0, ")).toBe(true);
  });

  it("falls back to the usual curve where the webview has no linear()", () => {
    expect(springCurve([520, 0.8], false).easing).toMatch(/^cubic-bezier/);
  });

  it("eases a Bézier curve from its start to its end", () => {
    const ease = bezier(0.23, 1, 0.32, 1);
    expect(ease(0)).toBeCloseTo(0);
    expect(ease(1)).toBeCloseTo(1);
    expect(ease(0.5)).toBeGreaterThan(0.8);
  });

  it("knows where a slide under way has got to", () => {
    const glide = MOTION_STYLES.glide;
    if (!glide) throw new Error("glide moves");
    const curve = slideCurve(glide);
    const slide = { from: -100, started: 0, curve };
    expect(slideOffset(slide, 0)).toBeCloseTo(-100);
    expect(Math.abs(slideOffset(slide, curve.duration / 2))).toBeLessThan(100);
    expect(slideOffset(slide, curve.duration)).toBe(0);
  });

  it("moves nothing in the instant style", () => {
    expect(MOTION_STYLES.instant).toBeNull();
  });
});

describe("the mark of the tab on show", () => {
  it("leads with the end facing the way it goes", () => {
    const glow = MOTION_STYLES.glow;
    if (!glow) throw new Error("glow moves");
    expect(springsFor(glow, true)).toEqual({ left: glow.trail, right: glow.lead });
    expect(springsFor(glow, false)).toEqual({ left: glow.lead, right: glow.trail });
  });

  it("stretches on the way and comes to rest on its tab", () => {
    const glow = MOTION_STYLES.glow;
    if (!glow) throw new Error("glow moves");
    const ends: Ends = { left: 0, right: 100, vLeft: 0, vRight: 0 };
    const target = { left: 300, right: 400 };
    const springs = springsFor(glow, true);
    let widest = 0;
    let frames = 0;
    while (!stepEnds(ends, target, springs, 1 / 60) && frames < 600) {
      widest = Math.max(widest, ends.right - ends.left);
      frames += 1;
    }
    expect(frames).toBeLessThan(600);
    expect(widest).toBeGreaterThan(100);
    expect(ends.left).toBeCloseTo(300, 0);
    expect(ends.right).toBeCloseTo(400, 0);
  });

  it("does not leap after a long pause between frames", () => {
    const ends: Ends = { left: 0, right: 100, vLeft: 0, vRight: 0 };
    stepEnds(ends, { left: 1000, right: 1100 }, { left: [600, 1], right: [600, 1] }, 5);
    expect(ends.left).toBeLessThan(1000);
  });

  it("reads the colours the theme writes", () => {
    const fallback = [1, 2, 3] as const;
    expect(rgbOf("#7fe3b8", fallback)).toEqual([127, 227, 184]);
    expect(rgbOf(" #7fe3b814 ", fallback)).toEqual([127, 227, 184]);
    expect(rgbOf("#fff", fallback)).toEqual([255, 255, 255]);
    expect(rgbOf("rgb(10, 20, 30)", fallback)).toEqual([10, 20, 30]);
    expect(rgbOf("rgb(10 20 30 / 0.5)", fallback)).toEqual([10, 20, 30]);
    expect(rgbOf("var(--x)", fallback)).toEqual(fallback);
  });

  it("blends from one colour to the next over the way", () => {
    expect(blend([0, 0, 0], [100, 200, 50], 0.5)).toEqual([50, 100, 25]);
    expect(blend([0, 0, 0], [100, 200, 50], 2)).toEqual([100, 200, 50]);
  });
});
