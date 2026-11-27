import { describe, expect, it } from "vitest";

import { easeOutCirc, easeOutCubic, fadeOutQuad, smoothstep } from "../src/fx/easing.ts";
import { CustomEffectError, type CustomDraw } from "../src/fx/custom.ts";
import { CursorEffects, type FxSurface } from "../src/fx/effects.ts";
import {
  convexHull,
  cornersOf,
  farEnough,
  pulseShape,
  startTrail,
  trailShape,
  warpShares,
  type FxRect,
} from "../src/fx/geometry.ts";
import { FX_FRAGMENT, FX_UNIFORMS, FX_VERTEX } from "../src/fx/shaders.ts";
import { ShapeKind, type FxBackend, type FxShape } from "../src/fx/shape.ts";
import {
  COMET_MS,
  FX_EDGE,
  FX_FIRST_FRAME_MS,
  FX_POLYGON_MAX,
  GLIDE_MS,
  GLIDE_SLOW_MS,
  PULSE_MS,
  WARP_TRAIL_SIZE,
} from "../src/fx/tuning.ts";
import { rgba } from "../src/frame.ts";

/** A cell of a 10 by 20 grid, in CSS pixels. */
const cell = (col: number, row: number): FxRect => ({
  x: col * 10,
  y: row * 20,
  width: 10,
  height: 20,
});

describe("the curves", () => {
  it("start at 0, end at 1 and hold outside", () => {
    for (const ease of [easeOutCirc, easeOutCubic]) {
      expect(ease(0)).toBe(0);
      expect(ease(1)).toBe(1);
      expect(ease(-1)).toBe(0);
      expect(ease(2)).toBe(1);
      // They settle: more than half way at the half.
      expect(ease(0.5)).toBeGreaterThan(0.5);
    }
    expect(fadeOutQuad(0)).toBe(1);
    expect(fadeOutQuad(1)).toBe(0);
    expect(smoothstep(0.3, 1, 0.3)).toBe(0);
    expect(smoothstep(0.3, 1, 1)).toBe(1);
  });
});

describe("the outline of a trail", () => {
  it("is the hull of two cells: a box going straight, six corners going aslant", () => {
    const out = new Float32Array(FX_POLYGON_MAX * 2);
    const straight = convexHull([...cornersOf(cell(0, 0)), ...cornersOf(cell(5, 0))], out);
    expect(straight).toBe(4);
    const aslant = convexHull([...cornersOf(cell(0, 0)), ...cornersOf(cell(5, 3))], out);
    expect(aslant).toBe(6);
    // The same cell twice is one cell.
    expect(convexHull([...cornersOf(cell(2, 2)), ...cornersOf(cell(2, 2))], out)).toBe(4);
  });

  it("goes round one way, with no edge of no length", () => {
    const out = new Float32Array(FX_POLYGON_MAX * 2);
    const count = convexHull([...cornersOf(cell(0, 0)), ...cornersOf(cell(4, 6))], out);
    let turning = 0;
    for (let i = 0; i < count; i++) {
      const j = (i + 1) % count;
      const k = (i + 2) % count;
      const ax = out[j * 2]! - out[i * 2]!;
      const ay = out[j * 2 + 1]! - out[i * 2 + 1]!;
      const bx = out[k * 2]! - out[j * 2]!;
      const by = out[k * 2 + 1]! - out[j * 2 + 1]!;
      expect(Math.hypot(ax, ay)).toBeGreaterThan(0);
      turning += Math.sign(ax * by - ay * bx);
    }
    expect(Math.abs(turning)).toBe(count);
  });

  it("needs a jump of more than a cursor and a half", () => {
    const from = cornersOf(cell(0, 0));
    expect(farEnough(from, cell(1, 0))).toBe(false);
    expect(farEnough(from, cell(0, 1))).toBe(false);
    expect(farEnough(from, cell(0, 2))).toBe(true);
    expect(farEnough(from, cell(30, 0))).toBe(true);
  });
});

describe("a warp", () => {
  it("brings the corners that lead first and those that trail last", () => {
    const lead = 1 - WARP_TRAIL_SIZE;
    const side = (lead + 1) / 2;
    // Corners: top left, top right, bottom right, bottom left.
    expect(warpShares(cornersOf(cell(0, 0)), cell(9, 0))).toEqual([1, lead, lead, 1]);
    expect(warpShares(cornersOf(cell(0, 5)), cell(0, 0))).toEqual([lead, lead, 1, 1]);
    expect(warpShares(cornersOf(cell(0, 0)), cell(9, 9))).toEqual([1, side, lead, side]);
  });

  it("starts as the old cell, ends as the new one, and is done then", () => {
    const out = new Float32Array(FX_POLYGON_MAX * 2);
    const trail = startTrail("warp", cornersOf(cell(0, 0)), cell(20, 0), 0, 200, 800);
    const first = trailShape(trail, 0, out);
    expect(first.count).toBe(4);
    expect([...out.slice(0, 8)].sort((a, b) => a - b)).toEqual([0, 0, 0, 0, 10, 10, 20, 20]);
    expect(first.done).toBe(false);
    // Half way, the leading edge has gone further than the trailing one.
    const half = trailShape(trail, 100, out);
    const xs = [...out.slice(0, half.count * 2)].filter((_, i) => i % 2 === 0);
    expect(Math.max(...xs) - Math.min(...xs)).toBeGreaterThan(10);
    expect(trailShape(trail, 200, out).done).toBe(true);
  });
});

describe("a comet", () => {
  it("puts its head in the new place at once on a long jump and lets the tail follow", () => {
    const out = new Float32Array(FX_POLYGON_MAX * 2);
    const trail = startTrail("comet", cornersOf(cell(0, 0)), cell(80, 30), 0, COMET_MS, 800);
    expect(trail.short).toBe(false);
    const shape = trailShape(trail, 0, out);
    expect(shape.head).toEqual({ x: 805, y: 610 });
    expect(shape.tail).toEqual({ x: 5, y: 10 });
    const later = trailShape(trail, COMET_MS / 2, out);
    expect(later.tail.x).toBeGreaterThan(5);
    expect(later.tail.x).toBeLessThan(805);
  });

  it("flies its head across on a short jump", () => {
    const out = new Float32Array(FX_POLYGON_MAX * 2);
    const trail = startTrail("comet", cornersOf(cell(0, 0)), cell(6, 0), 0, COMET_MS, 800);
    expect(trail.short).toBe(true);
    const shape = trailShape(trail, 0, out);
    expect(shape.head.x).toBe(5);
    expect(trailShape(trail, COMET_MS / 2, out).head.x).toBeGreaterThan(5);
  });
});

describe("a pulse", () => {
  it("grows from the cursor and fades out", () => {
    const pulse = { kind: "ripple" as const, start: 0, duration: 150, at: cell(2, 2) };
    const start = pulseShape(pulse, 0);
    expect(start.centre).toEqual({ x: 25, y: 50 });
    expect(start.radius).toBe(0);
    expect(start.fade).toBe(1);
    expect(start.ring).toBeGreaterThan(0);
    const later = pulseShape(pulse, 75);
    expect(later.radius).toBeGreaterThan(0);
    expect(later.fade).toBeLessThan(1);
    expect(pulseShape(pulse, 150).done).toBe(true);
    expect(pulseShape({ ...pulse, kind: "box-burst" }, 0).ring).toBe(0);
    expect(pulseShape({ ...pulse, kind: "box-burst" }, 0).halfWidth).toBe(5);
  });
});

describe("the WebGL2 shaders", () => {
  it("declare every uniform the backend sets, and no more", () => {
    const declared = [...`${FX_VERTEX}\n${FX_FRAGMENT}`.matchAll(/uniform \w+ (\w+)/g)].map(
      (match) => match[1],
    );
    expect(new Set(declared)).toEqual(new Set(FX_UNIFORMS));
  });

  it("write premultiplied colour and leave the cursor's own cell alone", () => {
    expect(FX_FRAGMENT).toContain("out_color = vec4(u_color * alpha, alpha)");
    expect(FX_FRAGMENT).toContain("discard");
  });
});

type Canvas = { id: number; visible: boolean; size?: { width: number; height: number } };

class FakeFx implements FxBackend {
  readonly kind: "webgl2" | "canvas2d";
  readonly canvas: Canvas;
  draws: FxShape[][] = [];
  clears = 0;
  disposed = false;
  pixels: [number, number] = [0, 0];
  lost: (() => void) | undefined;
  constructor(kind: "webgl2" | "canvas2d", canvas: Canvas) {
    this.kind = kind;
    this.canvas = canvas;
  }
  resize(width: number, height: number) {
    this.pixels = [width, height];
  }
  draw(shapes: readonly FxShape[], count: number) {
    // Copied: the effects fill the same records again next frame.
    this.draws.push(
      shapes.slice(0, count).map((shape) => ({ ...shape, points: shape.points.slice() })),
    );
  }
  clear() {
    this.clears++;
  }
  onLost(listener: () => void) {
    this.lost = listener;
  }
  dispose() {
    this.disposed = true;
  }
  /** A plugin's shaders, by id: being built, built, or the reason they cannot be. */
  built = new Map<string, "building" | "built" | string>();
  customs: string[] = [];
  prepareCustom(id: string) {
    if (!this.built.has(id)) this.built.set(id, "building");
  }
  customReady(id: string) {
    const state = this.built.get(id);
    if (state === undefined || state === "building") return false;
    if (state !== "built") throw new CustomEffectError(state);
    return true;
  }
  customDraws: CustomDraw[] = [];
  drawCustom(id: string, draw: CustomDraw) {
    this.customs.push(id);
    this.customDraws.push(draw);
  }
  forgetCustom(id: string) {
    this.built.delete(id);
  }
}

function setup(options: { webgl2?: "works" | "fails" | "none" } = {}) {
  let next = 0;
  const shown: Canvas[] = [];
  const made: FakeFx[] = [];
  let frames = 0;
  let disposed = 0;
  const surface: FxSurface<Canvas> = {
    createCanvas: () => ({ id: next++, visible: true }),
    show: (canvas) => void shown.push(canvas),
    remove: (canvas) => {
      const at = shown.indexOf(canvas);
      if (at >= 0) shown.splice(at, 1);
    },
    setCssSize: (canvas, size) => void (canvas.size = size),
    setVisible: (canvas, visible) => void (canvas.visible = visible),
  };
  const webgl2 = options.webgl2 ?? "works";
  const effects = new CursorEffects<Canvas>(
    surface,
    {
      webgl2:
        webgl2 === "none"
          ? undefined
          : (canvas) => {
              if (webgl2 === "fails") throw new Error("no WebGL2");
              const backend = new FakeFx("webgl2", canvas);
              made.push(backend);
              return backend;
            },
      canvas2d: (canvas) => {
        const backend = new FakeFx("canvas2d", canvas);
        made.push(backend);
        return backend;
      },
    },
    () => void frames++,
    () => void disposed++,
  );
  effects.resize({ width: 1000, height: 800 }, 2);
  return {
    effects,
    shown,
    made,
    frames: () => frames,
    disposed: () => disposed,
    last: () => made[made.length - 1]!,
  };
}

const MINT = rgba(127, 227, 184);

describe("the cursor's effects", () => {
  it("make nothing while every effect is off, and give it up when they are turned off", () => {
    const { effects, made, shown } = setup();
    effects.configure({ trail: "none", pulse: "none" });
    expect(made).toEqual([]);
    expect(effects.move(cell(0, 0), cell(50, 30), MINT, 0)).toBe(false);
    effects.configure({ trail: "comet", pulse: "none" });
    expect(effects.backend).toBe("webgl2");
    expect(shown.length).toBe(1);
    // Hidden until something plays, and sized to the window in device pixels.
    expect(shown[0]!.visible).toBe(false);
    expect(made[0]!.pixels).toEqual([2000, 1600]);
    effects.configure({ trail: "none", pulse: "none" });
    expect(made[0]!.disposed).toBe(true);
    expect(shown).toEqual([]);
    expect(effects.backend).toBeUndefined();
  });

  it("play a trail for a long jump in the frame it is found, then hide", () => {
    const { effects, last, frames, shown } = setup();
    effects.configure({ trail: "comet", pulse: "none" });
    expect(effects.move(cell(0, 0), cell(1, 0), MINT, 0)).toBe(false);
    expect(effects.needsFrame()).toBe(false);

    expect(effects.move(cell(0, 0), cell(50, 30), MINT, 1000)).toBe(true);
    expect(frames()).toBe(1);
    expect(effects.needsFrame()).toBe(true);
    effects.frame(1000);
    const [shape] = last().draws[0]!;
    expect(shape!.kind).toBe(ShapeKind.Polygon);
    expect(shape!.color).toBe(MINT);
    // In device pixels, with the cursor's own cell left clear.
    expect([shape!.holeLeft, shape!.holeTop, shape!.holeRight, shape!.holeBottom]).toEqual([
      1000 - FX_EDGE,
      1200 - FX_EDGE,
      1020 + FX_EDGE,
      1240 + FX_EDGE,
    ]);
    expect(shape!.haloRadius).toBeGreaterThan(0);
    expect(shown[0]!.visible).toBe(true);

    effects.frame(1000 + COMET_MS);
    expect(effects.needsFrame()).toBe(false);
    expect(shown[0]!.visible).toBe(false);
    expect(last().draws.length).toBe(1);
  });

  it("start in motion: the first frame is already a little way in", () => {
    const { effects, last } = setup();
    effects.configure({ trail: "warp", pulse: "none" });
    effects.move(cell(0, 0), cell(40, 0), MINT, 500);
    effects.frame(500);
    const xs = [...last().draws[0]![0]!.points.slice(0, 8)].filter((_, i) => i % 2 === 0);
    // Further than the old cell's right edge, in device pixels.
    expect(Math.max(...xs)).toBeGreaterThan(20);
    expect(FX_FIRST_FRAME_MS).toBeGreaterThan(0);
  });

  it("leave from where a trail under way has got to when the cursor jumps again", () => {
    const { effects, last } = setup();
    effects.configure({ trail: "sweep", pulse: "none" });
    effects.move(cell(0, 0), cell(60, 0), MINT, 0);
    effects.frame(60);
    effects.move(cell(60, 0), cell(60, 30), MINT, 60);
    effects.frame(60);
    const xs = [...last().draws[1]![0]!.points].filter((_, i) => i % 2 === 0);
    // The band starts between the first two places, not at the second.
    expect(Math.min(...xs.filter((x) => x > 0))).toBeLessThan(1200);
  });

  it("keep a trail going, aimed at the cursor, while it is typed after", () => {
    const { effects, last } = setup();
    effects.configure({ trail: "comet", pulse: "none" });
    effects.move(cell(0, 0), cell(50, 30), MINT, 0);
    effects.frame(10);
    expect(effects.move(cell(50, 30), cell(51, 30), MINT, 20)).toBe(false);
    effects.frame(20);
    const shape = last().draws[1]![0]!;
    expect(shape.holeLeft).toBe(1020 - FX_EDGE);
    // It still ends when the first would have.
    effects.frame(COMET_MS);
    expect(effects.needsFrame()).toBe(false);
  });

  it("pulse with the trail, in their own shapes", () => {
    const { effects, last } = setup();
    effects.configure({ trail: "warp", pulse: "box-ripple" });
    effects.move(cell(0, 0), cell(30, 20), MINT, 0);
    expect(effects.pulse(cell(30, 20), MINT, 0)).toBe(true);
    effects.frame(10);
    const [trail, pulse] = last().draws[0]!;
    expect(trail!.kind).toBe(ShapeKind.Polygon);
    expect(pulse!.kind).toBe(ShapeKind.BoxRing);
    expect(pulse!.alpha).toBeLessThan(1);
    effects.frame(PULSE_MS);
    expect(last().draws[1]!.length).toBe(1);
  });

  it("take the length the settings give", () => {
    const { effects } = setup();
    effects.configure({ trail: "comet", pulse: "none", length: 2.5 });
    effects.move(cell(0, 0), cell(50, 30), MINT, 0);
    effects.frame(COMET_MS + 10);
    expect(effects.needsFrame()).toBe(true);
    effects.frame(COMET_MS * 2.5);
    expect(effects.needsFrame()).toBe(false);
  });

  it("use Canvas 2D where WebGL2 cannot be had, on a canvas of its own", () => {
    const { effects, made, shown } = setup({ webgl2: "fails" });
    effects.configure({ trail: "comet", pulse: "none" });
    expect(effects.backend).toBe("canvas2d");
    expect(shown.length).toBe(1);
    expect(shown[0]).toBe(made[0]!.canvas);
    const none = setup({ webgl2: "none" });
    none.effects.configure({ trail: "none", pulse: "ripple" });
    expect(none.effects.backend).toBe("canvas2d");
  });

  it("go on with Canvas 2D when WebGL2 is lost, still playing", () => {
    const { effects, made, shown, frames } = setup();
    effects.configure({ trail: "comet", pulse: "none" });
    effects.move(cell(0, 0), cell(50, 30), MINT, 0);
    made[0]!.lost?.();
    expect(made[0]!.disposed).toBe(true);
    expect(effects.backend).toBe("canvas2d");
    expect(shown.length).toBe(1);
    expect(shown[0]).toBe(made[1]!.canvas);
    expect(frames()).toBeGreaterThan(0);
  });

  it("stop at once when asked, and say when they are given up", () => {
    const { effects, shown, disposed } = setup();
    effects.configure({ trail: "comet", pulse: "burst" });
    effects.move(cell(0, 0), cell(50, 30), MINT, 0);
    effects.pulse(cell(50, 30), MINT, 0);
    effects.frame(10);
    effects.cancel();
    expect(effects.needsFrame()).toBe(false);
    expect(shown[0]!.visible).toBe(false);
    effects.dispose();
    expect(disposed()).toBe(1);
    expect(shown).toEqual([]);
  });
});

describe("the cursor's own motion", () => {
  it("draws the cursor as a box sliding from its old place to its new, then gives it back", () => {
    const { effects, last } = setup();
    effects.configure({ trail: "none", pulse: "none", motion: "glide" });
    expect(effects.cursorDrawnUntil).toBe(-Infinity);
    // A bar one cell on, as when a key is typed: drawn in device pixels, twice CSS ones.
    const bar = (col: number): FxRect => ({ x: col * 10, y: 20, width: 2, height: 20 });
    expect(effects.glide(bar(3), bar(4), MINT, 100)).toBe(true);
    const until = 100 - FX_FIRST_FRAME_MS + GLIDE_MS;
    expect(effects.cursorDrawnUntil).toBe(until);
    effects.frame(100);
    const box = last().draws.at(-1)![0]!;
    expect(box.kind).toBe(ShapeKind.Box);
    expect(box.halfWidth).toBe(2);
    expect(box.halfHeight).toBe(20);
    // Quick to leave: a little way in, more than that share of the way is behind it.
    expect(box.centreX).toBeGreaterThan(62 + 20 * (FX_FIRST_FRAME_MS / GLIDE_MS));
    expect(box.centreX).toBeLessThan(82);
    expect(box.holeRight).toBe(0);
    effects.frame(until - 0.001);
    expect(last().draws.at(-1)![0]!.centreX).toBeCloseTo(82, 1);
    // The frame that reaches the end draws nothing: the pane draws the cursor again.
    effects.frame(until);
    expect(effects.cursorDrawnUntil).toBe(-Infinity);
    expect(effects.playing).toBe(false);
  });

  it("plays where WebGL2 cannot be had, takes the settings' length, and stops when turned off", () => {
    const { effects, last } = setup({ webgl2: "none" });
    effects.configure({ trail: "none", pulse: "none", motion: "glide-slow", length: 2 });
    expect(last().kind).toBe("canvas2d");
    expect(effects.glide(cell(0, 0), cell(1, 0), MINT, 0)).toBe(true);
    expect(effects.cursorDrawnUntil).toBe(-FX_FIRST_FRAME_MS + GLIDE_SLOW_MS * 2);
    effects.configure({ trail: "none", pulse: "none" });
    expect(effects.cursorDrawnUntil).toBe(-Infinity);
    expect(effects.glide(cell(0, 0), cell(1, 0), MINT, 10)).toBe(false);
  });

  it("gives way to a plugin's motion once that is ready", () => {
    const { effects, last } = setup();
    const glow = { id: "acme/glide", source: "x", duration: 300, reach: 8 };
    effects.configure({ trail: "none", pulse: "none", motion: "glide", customMotion: glow });
    // Still being built: Lume's own plays meanwhile.
    expect(effects.glide(cell(0, 0), cell(1, 0), MINT, 0)).toBe(true);
    effects.frame(5);
    expect(last().customs).toEqual([]);
    last().built.set("acme/glide", "built");
    expect(effects.glide(cell(1, 0), cell(2, 0), MINT, 20)).toBe(true);
    effects.frame(25);
    expect(last().customs).toEqual(["acme/glide"]);
    expect(last().draws.at(-1)).toHaveLength(0);
  });
});

describe("a plugin's cursor effect", () => {
  const glow = { id: "acme/glow", source: "x", duration: 300, reach: 32 };

  it("is not played, and nothing waits, while its shader is being built", () => {
    const { effects, last, frames } = setup();
    effects.configure({ trail: "none", pulse: "none", customTrail: glow });
    expect(last().built.get("acme/glow")).toBe("building");
    // The cursor jumps before the shader is there: the jump goes without.
    expect(effects.move(cell(0, 0), cell(50, 30), MINT, 0)).toBe(false);
    expect(frames()).toBe(0);
    expect(effects.playing).toBe(false);

    // Built: the next jump plays it.
    last().built.set("acme/glow", "built");
    expect(effects.move(cell(0, 0), cell(50, 30), MINT, 100)).toBe(true);
    effects.frame(110);
    expect(last().customs).toEqual(["acme/glow"]);
  });

  it("as the cursor's motion, plays on every step, over the cursor's place, until it says", () => {
    const { effects, last } = setup();
    effects.configure({ trail: "none", pulse: "none", customMotion: glow });
    last().built.set("acme/glow", "built");
    expect(effects.cursorDrawnUntil).toBe(-Infinity);
    // A step of one cell, which no trail would play for.
    expect(effects.glide(cell(3, 2), cell(4, 2), MINT, 100)).toBe(true);
    const until = 100 - FX_FIRST_FRAME_MS + 300;
    expect(effects.cursorDrawnUntil).toBe(until);
    effects.frame(110);
    expect(last().customs).toEqual(["acme/glow"]);
    const draw = last().customDraws[0]!;
    // In device pixels, and nothing is left clear: the motion draws the cursor itself.
    expect(draw.from).toEqual([60, 80, 20, 40]);
    expect(draw.to).toEqual([80, 80, 20, 40]);
    expect(draw.hole).toEqual([0, 0, 0, 0]);
    // The frame that reaches the end draws it no more, and the pane draws the cursor.
    effects.frame(until);
    expect(last().customs).toEqual(["acme/glow"]);
    expect(effects.cursorDrawnUntil).toBe(-Infinity);
    expect(effects.playing).toBe(false);
  });

  it("as the cursor's motion, plays beside a trail, and ends when it is no longer chosen", () => {
    const { effects, last } = setup();
    effects.configure({ trail: "comet", pulse: "none", customMotion: glow });
    last().built.set("acme/glow", "built");
    expect(effects.move(cell(0, 0), cell(50, 30), MINT, 0)).toBe(true);
    expect(effects.glide(cell(0, 0), cell(50, 30), MINT, 0)).toBe(true);
    effects.frame(10);
    expect(last().draws.at(-1)).toHaveLength(1);
    expect(last().customs).toEqual(["acme/glow"]);
    effects.configure({ trail: "comet", pulse: "none" });
    expect(effects.cursorDrawnUntil).toBe(-Infinity);
    expect(effects.glide(cell(0, 0), cell(1, 0), MINT, 20)).toBe(false);
  });

  it("is given up and said once when its shader cannot be built", () => {
    const { effects, last } = setup();
    const problems: [string, string][] = [];
    effects.onProblem((id, message) => problems.push([id, message]));
    effects.configure({ trail: "none", pulse: "none", customPulse: glow });
    last().built.set("acme/glow", "0:3: syntax error");
    expect(effects.pulse(cell(4, 4), MINT, 0)).toBe(false);
    expect(effects.pulse(cell(4, 4), MINT, 10)).toBe(false);
    expect(problems).toEqual([["acme/glow", "0:3: syntax error"]]);
    expect(last().customs).toEqual([]);
  });
});
