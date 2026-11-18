/**
 * The cursor's effects in the snapshot suite: each effect frozen at one moment, drawn
 * with WebGL2 and with Canvas 2D.
 *
 * The WebGL2 picture is held to a reference kept for the engine, as the frames of the
 * terminal are. The two ways of drawing are not the same picture (a glow from a shader
 * and the canvas's own shadow differ), so the Canvas 2D one is held to less: it draws
 * something, and it leaves the cursor's own cell clear, as WebGL2 must too.
 */

import {
  Canvas2dFx,
  CursorEffects,
  rgba,
  WebGl2Fx,
  type CursorEffectSettings,
  type FxRect,
  type FxSurface,
} from "@lume/renderer";

import { readPixels } from "./pixels.ts";
import { FX_SNAPSHOT_HEIGHT_PX, FX_SNAPSHOT_MS, FX_SNAPSHOT_WIDTH_PX } from "./tuning.ts";

/** An effect, frozen. */
export interface FxPicture {
  readonly name: string;
  /** Each backend's picture, by its name. */
  readonly pictures: ReadonlyMap<string, ImageData>;
  readonly problems: readonly string[];
}

const EFFECTS: readonly CursorEffectSettings[] = [
  { trail: "warp", pulse: "none" },
  { trail: "sweep", pulse: "none" },
  { trail: "comet", pulse: "none" },
  { trail: "none", pulse: "burst" },
  { trail: "none", pulse: "ripple" },
  { trail: "none", pulse: "box-burst" },
  { trail: "none", pulse: "box-ripple" },
];

function cell(col: number, row: number): FxRect {
  return { x: col * 9, y: row * 18, width: 9, height: 18 };
}

const FROM = cell(2, 1);
const TO = cell(32, 7);
const COLOR = rgba(122, 167, 255);

/** How many pixels of a picture have any colour, and how many of them are in `rect`. */
function inked(picture: ImageData, rect: FxRect, dpr: number): { all: number; inside: number } {
  const left = Math.round(rect.x * dpr);
  const top = Math.round(rect.y * dpr);
  const right = Math.round((rect.x + rect.width) * dpr);
  const bottom = Math.round((rect.y + rect.height) * dpr);
  let all = 0;
  let inside = 0;
  for (let y = 0; y < picture.height; y++) {
    for (let x = 0; x < picture.width; x++) {
      if ((picture.data[(y * picture.width + x) * 4 + 3] ?? 0) === 0) continue;
      all++;
      if (x >= left && x < right && y >= top && y < bottom) inside++;
    }
  }
  return { all, inside };
}

/** Draws one effect with one backend, frozen; nothing if the backend is not there. */
function draw(
  parent: HTMLElement,
  kind: "webgl2" | "canvas2d",
  settings: CursorEffectSettings,
  dpr: number,
): ImageData | undefined {
  let canvas: HTMLCanvasElement | undefined;
  const surface: FxSurface<HTMLCanvasElement> = {
    createCanvas: () => document.createElement("canvas"),
    show(shown) {
      canvas = shown;
      parent.replaceChildren(shown);
    },
    remove: (gone) => gone.remove(),
    setCssSize(on, size) {
      on.style.width = `${size.width}px`;
      on.style.height = `${size.height}px`;
    },
    setVisible: () => undefined,
  };
  const effects = new CursorEffects<HTMLCanvasElement>(
    surface,
    {
      webgl2: kind === "webgl2" ? (on) => new WebGl2Fx(on) : undefined,
      canvas2d: (on) => new Canvas2dFx(on),
    },
    () => undefined,
  );
  try {
    effects.resize({ width: FX_SNAPSHOT_WIDTH_PX, height: FX_SNAPSHOT_HEIGHT_PX }, dpr);
    effects.configure(settings);
    if (effects.backend !== kind || !canvas) return undefined;
    if (settings.trail !== "none") effects.move(FROM, TO, COLOR, 0);
    else effects.pulse(TO, COLOR, 0);
    effects.frame(FX_SNAPSHOT_MS);
    // Read in the task that drew: a GPU canvas keeps its picture only until then.
    return readPixels(canvas);
  } finally {
    effects.dispose();
  }
}

/** Every effect, frozen, with both backends. */
export function drawCursorFx(dpr: number): FxPicture[] {
  const parent = document.createElement("div");
  parent.className = "harness-surface";
  document.body.append(parent);
  try {
    return EFFECTS.map((settings) => {
      const trail = settings.trail !== "none";
      const name = `fx: ${trail ? settings.trail : settings.pulse}`;
      const pictures = new Map<string, ImageData>();
      const problems: string[] = [];
      for (const kind of ["webgl2", "canvas2d"] as const) {
        const picture = draw(parent, kind, settings, dpr);
        if (!picture) continue;
        pictures.set(kind, picture);
        const { all, inside } = inked(picture, TO, dpr);
        if (all === 0) problems.push(`${kind} drew nothing`);
        // A trail leaves the cell the cursor is in to the cursor itself.
        if (trail && inside > 0) problems.push(`${kind} drew over the cursor's cell`);
      }
      if (!pictures.has("canvas2d")) problems.push("Canvas 2D could not draw the effects");
      return { name, pictures, problems };
    });
  } finally {
    parent.remove();
  }
}
