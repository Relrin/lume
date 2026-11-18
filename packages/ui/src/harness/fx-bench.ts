/**
 * What drawing the cursor's effects costs, alone: every effect drawn frame after frame
 * over a window-sized canvas, with WebGL2 and with Canvas 2D, each frame timed until
 * it was handed over and until the GPU had finished it.
 *
 * The clock the effects are drawn at is the benchmark's own, stepped through each
 * effect from its start to its end, so every frame draws an effect part way through,
 * the most an effect ever draws.
 */

import {
  Canvas2dFx,
  CursorEffects,
  rgba,
  WebGl2Fx,
  type CursorEffectSettings,
  type FxRect,
  type FxSurface,
  type PulseKind,
  type TrailKind,
} from "@lume/renderer";

import { nextFrame, timing } from "./timing.ts";
import {
  FX_EFFECT_SPAN_MS,
  FX_FRAMES,
  FX_FRAMES_PER_EFFECT,
  FX_SURFACE_HEIGHT_PX,
  FX_SURFACE_WIDTH_PX,
  FX_WARMUP_FRAMES,
} from "./tuning.ts";
import type { RedrawTiming } from "./bench.ts";

/** What draws the effects. */
export type FxBackendKind = "webgl2" | "canvas2d";

/** Each effect's frames, by what drew them and then by the effect. */
export type FxBench = Partial<Record<FxBackendKind, Record<string, RedrawTiming>>>;

const TRAILS: readonly Exclude<TrailKind, "none">[] = ["warp", "sweep", "comet"];
const PULSES: readonly Exclude<PulseKind, "none">[] = [
  "burst",
  "ripple",
  "box-burst",
  "box-ripple",
];

/** A cell of a 9 by 18 pixel grid, the size of a terminal's cells at 15 px. */
function cell(col: number, row: number): FxRect {
  return { x: 40 + col * 9, y: 60 + row * 18, width: 9, height: 18 };
}

/** The places a trail jumps between: across the window, down it, and aslant. */
const JUMPS: readonly (readonly [FxRect, FxRect])[] = [
  [cell(2, 2), cell(170, 2)],
  [cell(170, 2), cell(10, 50)],
  [cell(10, 50), cell(90, 20)],
  [cell(90, 20), cell(2, 2)],
];

const COLOR = rgba(127, 227, 184);

function surfaceIn(parent: HTMLElement): FxSurface<HTMLCanvasElement> {
  return {
    createCanvas() {
      const canvas = document.createElement("canvas");
      canvas.style.display = "block";
      return canvas;
    },
    show(canvas) {
      parent.replaceChildren(canvas);
    },
    remove(canvas) {
      canvas.remove();
    },
    setCssSize(canvas, size) {
      canvas.style.width = `${size.width}px`;
      canvas.style.height = `${size.height}px`;
    },
    setVisible(canvas, visible) {
      canvas.style.visibility = visible ? "visible" : "hidden";
    },
  };
}

/** Times one effect drawn with one backend. Returns nothing if the backend is not there. */
async function timeOne(
  parent: HTMLElement,
  kind: FxBackendKind,
  settings: CursorEffectSettings,
  probe: CanvasRenderingContext2D,
): Promise<RedrawTiming | undefined> {
  let canvas: HTMLCanvasElement | undefined;
  const effects = new CursorEffects<HTMLCanvasElement>(
    {
      ...surfaceIn(parent),
      show(shown) {
        canvas = shown;
        parent.replaceChildren(shown);
      },
    },
    {
      webgl2: kind === "webgl2" ? (on) => new WebGl2Fx(on) : undefined,
      canvas2d: (on) => new Canvas2dFx(on),
    },
    () => undefined,
  );
  try {
    effects.resize(
      { width: FX_SURFACE_WIDTH_PX, height: FX_SURFACE_HEIGHT_PX },
      window.devicePixelRatio,
    );
    effects.configure(settings);
    if (effects.backend !== kind || !canvas) return undefined;
    const draw: number[] = [];
    const finish: number[] = [];
    const trail = settings.trail !== "none";
    let clock = 0;
    let start = 0;
    let jump = 0;
    for (let i = -FX_WARMUP_FRAMES; i < FX_FRAMES; i++) {
      const step = (i + FX_WARMUP_FRAMES) % FX_FRAMES_PER_EFFECT;
      if (step === 0) {
        start = clock;
        const [from, to] = JUMPS[jump++ % JUMPS.length]!;
        if (trail) effects.move(from, to, COLOR, start);
        else effects.pulse(to, COLOR, start);
      }
      // Through the effect from its start to before its end.
      clock = start + ((step + 0.5) * FX_EFFECT_SPAN_MS) / FX_FRAMES_PER_EFFECT;
      const began = performance.now();
      effects.frame(clock);
      const drawn = performance.now();
      // Reading a pixel back makes the webview wait for the GPU to finish the frame.
      probe.drawImage(canvas, 0, 0, 1, 1, 0, 0, 1, 1);
      probe.getImageData(0, 0, 1, 1);
      const finished = performance.now();
      if (i >= 0) {
        draw.push(drawn - began);
        finish.push(finished - began);
      }
    }
    return { draw: timing(draw), finish: timing(finish) };
  } finally {
    effects.dispose();
    await nextFrame();
  }
}

/** Every effect drawn with every backend the webview offers. */
export async function timeCursorFx(parent: HTMLElement): Promise<FxBench> {
  const surface = document.createElement("div");
  surface.className = "harness-surface";
  parent.append(surface);
  const probeCanvas = document.createElement("canvas");
  probeCanvas.width = 1;
  probeCanvas.height = 1;
  const probe = probeCanvas.getContext("2d", { willReadFrequently: true });
  if (!probe) throw new Error("no Canvas 2D context to read pixels back with");
  const report: FxBench = {};
  try {
    for (const kind of ["webgl2", "canvas2d"] as const) {
      const effects: Record<string, RedrawTiming> = {};
      const all: CursorEffectSettings[] = [
        ...TRAILS.map((trail) => ({ trail, pulse: "none" as const })),
        ...PULSES.map((pulse) => ({ trail: "none" as const, pulse })),
      ];
      for (const settings of all) {
        const name = settings.trail !== "none" ? settings.trail : settings.pulse;
        const timed = await timeOne(surface, kind, settings, probe);
        if (timed) effects[name] = timed;
      }
      if (Object.keys(effects).length > 0) report[kind] = effects;
    }
  } finally {
    surface.remove();
  }
  return report;
}
