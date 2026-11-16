import { describe, expect, it } from "vitest";

import { FrameLoop, type FrameLoopEnvironment, type FrameView } from "../src/loop.ts";
import {
  ManagedRenderer,
  TierUnavailableError,
  type Surface,
  type TierFactory,
} from "../src/select.ts";
import { HighlightKind } from "../src/frame.ts";
import {
  FakeFonts,
  highlights,
  makeFrame,
  markDrawn,
  putText,
  RecordingBackend,
  TEST_THEME,
} from "../src/testing/index.ts";
import type { Size, Tier } from "../src/types.ts";

/** A canvas as far as tier selection cares: something with an identity. */
interface FakeCanvas {
  id: number;
  css?: Size;
}

class FakeSurface implements Surface<FakeCanvas> {
  created = 0;
  shown: FakeCanvas | undefined;
  readonly removed: number[] = [];

  createCanvas(): FakeCanvas {
    return { id: ++this.created };
  }
  show(canvas: FakeCanvas): void {
    this.shown = canvas;
  }
  remove(canvas: FakeCanvas): void {
    this.removed.push(canvas.id);
    if (this.shown === canvas) this.shown = undefined;
  }
  setCssSize(canvas: FakeCanvas, size: Size): void {
    canvas.css = size;
  }
}

/** Factories for the three tiers, each recording the backends it made. */
function tiers(unavailable: Tier[] = [], failingDraw: Tier[] = []) {
  const made: Record<Tier, RecordingBackend[]> = { webgpu: [], webgl2: [], canvas2d: [] };
  const canvases: Record<Tier, FakeCanvas[]> = { webgpu: [], webgl2: [], canvas2d: [] };
  const factory = (tier: Tier): TierFactory<FakeCanvas> => ({
    tier,
    create(canvas) {
      if (unavailable.includes(tier)) throw new TierUnavailableError(tier, "test");
      const backend = new RecordingBackend(tier);
      if (failingDraw.includes(tier)) {
        backend.present = () => {
          throw new Error(`${tier} cannot draw`);
        };
      }
      made[tier].push(backend);
      canvases[tier].push(canvas);
      return backend;
    },
  });
  return { made, canvases, factories: [factory("webgpu"), factory("webgl2"), factory("canvas2d")] };
}

function frame() {
  const f = makeFrame(40, 10);
  putText(f, 0, 0, "hello");
  return f;
}

describe("choosing a tier", () => {
  it("uses the first tier in the order", () => {
    const surface = new FakeSurface();
    const { factories, canvases } = tiers();
    const renderer = new ManagedRenderer(surface, factories, new FakeFonts(), TEST_THEME);
    expect(renderer.tier).toBe("webgpu");
    expect(surface.shown).toBe(canvases.webgpu[0]);
    expect(surface.created).toBe(1);
  });

  it("skips tiers that are not available and removes their unused canvases", () => {
    const surface = new FakeSurface();
    const { factories, canvases } = tiers(["webgpu", "webgl2"]);
    const renderer = new ManagedRenderer(surface, factories, new FakeFonts(), TEST_THEME);
    expect(renderer.tier).toBe("canvas2d");
    expect(surface.shown).toBe(canvases.canvas2d[0]);
    expect(surface.removed).toEqual([1, 2]);
  });

  it("uses only the tier it is told to", () => {
    const surface = new FakeSurface();
    const { factories } = tiers();
    const renderer = new ManagedRenderer(surface, factories, new FakeFonts(), TEST_THEME, {
      tier: "webgl2",
    });
    expect(renderer.tier).toBe("webgl2");
  });

  it("fails when no tier can be started", () => {
    const { factories } = tiers(["webgpu", "webgl2", "canvas2d"]);
    expect(
      () => new ManagedRenderer(new FakeSurface(), factories, new FakeFonts(), TEST_THEME),
    ).toThrow(/no renderer tier/);
    const forced = tiers(["webgl2"]);
    expect(
      () =>
        new ManagedRenderer(new FakeSurface(), forced.factories, new FakeFonts(), TEST_THEME, {
          tier: "webgl2",
        }),
    ).toThrow(/tried: webgl2/);
  });

  it("passes on errors that are not about availability", () => {
    const broken: TierFactory<FakeCanvas> = {
      tier: "webgpu",
      create() {
        throw new TypeError("a bug");
      },
    };
    expect(
      () => new ManagedRenderer(new FakeSurface(), [broken], new FakeFonts(), TEST_THEME),
    ).toThrow(TypeError);
  });

  it("sizes the canvas in CSS pixels and the tier in device pixels", () => {
    const surface = new FakeSurface();
    const { factories, made } = tiers();
    const renderer = new ManagedRenderer(surface, factories, new FakeFonts(), TEST_THEME);
    renderer.resize({ width: 320, height: 160 }, 2);
    renderer.draw(frame());
    expect(surface.shown!.css).toEqual({ width: 320, height: 160 });
    expect(made.webgpu[0]!.configured).toEqual([[640, 320]]);
  });
});

describe("losing a tier", () => {
  function started(unavailable: Tier[] = [], failingDraw: Tier[] = []) {
    const surface = new FakeSurface();
    const all = tiers(unavailable, failingDraw);
    const events: string[] = [];
    let invalidated = 0;
    const renderer = new ManagedRenderer(surface, all.factories, new FakeFonts(), TEST_THEME, {
      onTierChange: (tier, reason) => events.push(`${tier ?? "none"}: ${reason}`),
      onInvalidate: () => invalidated++,
    });
    renderer.resize({ width: 320, height: 160 }, 1);
    renderer.setFocused(false);
    return { surface, renderer, events, invalidated: () => invalidated, ...all };
  }

  it("asks for a frame, then draws it with the next tier and swaps the canvas", () => {
    const { surface, renderer, made, canvases, events, invalidated } = started();
    const f = frame();
    renderer.draw(f);
    markDrawn(f);

    made.webgpu[0]!.lose("device lost");
    expect(invalidated()).toBe(1);
    // Nothing changes until there is a frame to draw.
    expect(renderer.tier).toBe("webgpu");
    expect(surface.shown).toBe(canvases.webgpu[0]);

    // The frame says nothing changed, but the new tier has drawn nothing yet.
    renderer.draw(f);
    expect(renderer.tier).toBe("webgl2");
    expect(made.webgl2[0]!.presents.length).toBe(1);
    expect(made.webgl2[0]!.last.repaint).toBeUndefined();
    expect(surface.shown).toBe(canvases.webgl2[0]);
    expect(made.webgpu[0]!.disposed).toBe(true);
    expect(surface.removed).toContain(canvases.webgpu[0]!.id);
    expect(events).toEqual(["webgl2: device lost"]);
  });

  it("carries the size, the pixel ratio and the focus over to the next tier", () => {
    const { renderer, made, surface } = started();
    renderer.resize({ width: 400, height: 200 }, 2);
    const f = frame();
    renderer.draw(f);
    made.webgpu[0]!.lose();
    renderer.draw(f);
    expect(made.webgl2[0]!.configured).toEqual([[800, 400]]);
    expect(surface.shown!.css).toEqual({ width: 400, height: 200 });
    // Unfocused: the cursor would be hollow. The setting itself is what is checked.
    expect(renderer.metrics.cellWidth).toBe(16);
  });

  it("carries a hidden cursor and the underlined runs over to the next tier", () => {
    const { renderer, made } = started();
    const f = frame();
    f.cursor.visible = true;
    renderer.setCursorVisible(false);
    renderer.setUnderlines([{ row: 1, startCol: 0, endCol: 3 }]);
    renderer.draw(f);
    expect(made.webgpu[0]!.last.overlayRects).toBe(1);
    made.webgpu[0]!.lose();
    renderer.draw(f);
    // One rectangle: the run. The cursor, which would add four, stays hidden.
    expect(made.webgl2[0]!.last.overlayRects).toBe(1);
  });

  it("carries the highlighted runs over to the next tier", () => {
    const { renderer, made } = started();
    const f = frame();
    const runs = highlights([[0, 0, 0, HighlightKind.Active]]);
    renderer.setHighlights(runs);
    // The caller refills its list; what was given is what is kept.
    runs.count = 0;
    renderer.draw(f);
    made.webgpu[0]!.lose();
    renderer.draw(f);
    const background = made.webgl2[0]!.planes!.background;
    const color = new DataView(background.bytes.buffer).getUint32(8, true);
    expect(color).toBe(TEST_THEME.searchActive);
  });

  it("falls through a tier that cannot be started", () => {
    const { renderer, made, events } = started(["webgl2"]);
    const f = frame();
    renderer.draw(f);
    made.webgpu[0]!.lose("gone");
    renderer.draw(f);
    expect(renderer.tier).toBe("canvas2d");
    expect(events).toEqual(["canvas2d: gone"]);
  });

  it("treats a tier that throws while drawing as lost, in the same frame", () => {
    const { renderer, made, events } = started([], ["webgpu"]);
    renderer.draw(frame());
    expect(renderer.tier).toBe("webgl2");
    expect(made.webgl2[0]!.presents.length).toBe(1);
    expect(events).toEqual(["webgl2: webgpu cannot draw"]);
  });

  it("skips a next tier that also fails to draw", () => {
    const { renderer, made, surface, canvases } = started([], ["webgpu", "webgl2"]);
    renderer.draw(frame());
    expect(renderer.tier).toBe("canvas2d");
    expect(made.webgl2[0]!.disposed).toBe(true);
    expect(surface.removed).toContain(canvases.webgl2[0]!.id);
    expect(surface.shown).toBe(canvases.canvas2d[0]);
  });

  it("can lose one tier after another", () => {
    const { renderer, made } = started();
    const f = frame();
    renderer.draw(f);
    made.webgpu[0]!.lose();
    renderer.draw(f);
    made.webgl2[0]!.lose();
    renderer.draw(f);
    expect(renderer.tier).toBe("canvas2d");
  });

  it("keeps the last picture and says so when no tier is left", () => {
    const { renderer, made, surface, canvases, events } = started(["webgl2", "canvas2d"]);
    const f = frame();
    renderer.draw(f);
    made.webgpu[0]!.lose("gone");
    renderer.draw(f);
    renderer.draw(f);
    expect(events).toEqual(["none: gone"]);
    expect(surface.shown).toBe(canvases.webgpu[0]);
  });

  it("ignores a late loss report from a tier already replaced", () => {
    const { renderer, made, invalidated } = started();
    const f = frame();
    renderer.draw(f);
    made.webgpu[0]!.lose();
    renderer.draw(f);
    made.webgpu[0]!.lose("again");
    expect(invalidated()).toBe(1);
    renderer.draw(f);
    expect(renderer.tier).toBe("webgl2");
  });

  it("removes its canvas when disposed", () => {
    const { renderer, surface, made } = started();
    renderer.dispose();
    renderer.dispose();
    expect(surface.shown).toBeUndefined();
    expect(made.webgpu[0]!.disposed).toBe(true);
    renderer.draw(frame());
    expect(made.webgpu[0]!.presents).toEqual([]);
  });
});

describe("the frame loop", () => {
  function environment() {
    const pending = new Map<number, (now: number) => void>();
    const errors: unknown[] = [];
    let next = 1;
    let hidden = false;
    let visibility: (() => void) | undefined;
    const env: FrameLoopEnvironment = {
      requestFrame: (callback) => {
        pending.set(next, callback);
        return next++;
      },
      cancelFrame: (handle) => void pending.delete(handle),
      isHidden: () => hidden,
      onVisibilityChange: (listener) => {
        visibility = listener;
        return () => (visibility = undefined);
      },
      onError: (error) => errors.push(error),
    };
    return {
      env,
      errors,
      pending,
      /** Run the animation frame that was asked for. */
      tick(now = 16) {
        const callbacks = [...pending.values()];
        pending.clear();
        for (const callback of callbacks) callback(now);
      },
      setHidden(value: boolean) {
        hidden = value;
        visibility?.();
      },
      listening: () => visibility !== undefined,
    };
  }

  function view(dirty = true) {
    const v = {
      dirty,
      frames: [] as number[],
      needsFrame: () => v.dirty,
      frame(now: number) {
        v.frames.push(now);
        v.dirty = false;
      },
    };
    return v satisfies FrameView;
  }

  it("draws a dirty view on the next animation frame and then rests", () => {
    const { env, pending, tick } = environment();
    const loop = new FrameLoop(env);
    const v = view();
    loop.add(v);
    expect(pending.size).toBe(1);
    tick(100);
    expect(v.frames).toEqual([100]);
    expect(pending.size).toBe(0);
  });

  it("asks for one animation frame however often it is asked", () => {
    const { env, pending } = environment();
    const loop = new FrameLoop(env);
    loop.requestFrame();
    loop.requestFrame();
    loop.requestFrame();
    expect(pending.size).toBe(1);
  });

  it("skips a view with nothing new", () => {
    const { env, tick } = environment();
    const loop = new FrameLoop(env);
    const clean = view(false);
    const dirty = view();
    loop.add(clean);
    loop.add(dirty);
    tick();
    expect(clean.frames).toEqual([]);
    expect(dirty.frames.length).toBe(1);
  });

  it("keeps going while a view stays dirty", () => {
    const { env, pending, tick } = environment();
    const loop = new FrameLoop(env);
    const animating = {
      left: 3,
      needsFrame: () => animating.left > 0,
      frame: () => void animating.left--,
    };
    loop.add(animating);
    tick();
    expect(pending.size).toBe(1);
    tick();
    tick();
    expect(animating.left).toBe(0);
    expect(pending.size).toBe(0);
  });

  it("does not draw while hidden and catches up when shown", () => {
    const { env, pending, tick, setHidden } = environment();
    const loop = new FrameLoop(env);
    const v = view(false);
    loop.add(v);
    tick();
    setHidden(true);
    v.dirty = true;
    loop.requestFrame();
    expect(pending.size).toBe(0);
    setHidden(false);
    expect(pending.size).toBe(1);
    tick();
    expect(v.frames.length).toBe(1);
  });

  it("reports a view that throws and still draws the others", () => {
    const { env, errors, tick } = environment();
    const loop = new FrameLoop(env);
    const failing: FrameView = {
      needsFrame: () => true,
      frame: () => {
        throw new Error("draw failed");
      },
    };
    const fine = view();
    loop.add(failing);
    loop.add(fine);
    tick();
    expect(errors.length).toBe(1);
    expect(fine.frames.length).toBe(1);
  });

  it("stops drawing a removed view", () => {
    const { env, tick } = environment();
    const loop = new FrameLoop(env);
    const v = view();
    loop.add(v);
    loop.remove(v);
    tick();
    expect(v.frames).toEqual([]);
  });

  it("cancels its frame and stops listening when disposed", () => {
    const { env, pending, listening } = environment();
    const loop = new FrameLoop(env);
    loop.add(view());
    loop.dispose();
    loop.dispose();
    expect(pending.size).toBe(0);
    expect(listening()).toBe(false);
    loop.requestFrame();
    expect(pending.size).toBe(0);
  });
});
