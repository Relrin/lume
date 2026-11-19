/**
 * The cursor's effects for one window: a trail when the cursor jumps, a pulse when it
 * changes shape or lands in another pane.
 *
 * They are drawn on one transparent canvas over the whole window rather than by each
 * pane's renderer, so a trail can fly from one pane to another. The canvas is hidden
 * whenever nothing is playing: an idle window has no layer more to put together and
 * draws nothing. While something plays, the window's frame loop draws it once a frame,
 * after the panes, so a jump a pane found in a frame is shown in that same frame.
 */

import type { FrameView } from "../loop.ts";
import type { Size } from "../types.ts";
import { CustomEffectError, type CustomEffect } from "./custom.ts";
import {
  cornersOf,
  farEnough,
  pulseShape,
  startTrail,
  trailShape,
  type Corners,
  type FxRect,
  type Pulse,
  type PulseKind,
  type Trail,
  type TrailKind,
} from "./geometry.ts";
import { FxShape, ShapeKind, type FxBackend } from "./shape.ts";
import {
  COMET_FADE_POWER,
  COMET_HALO_RADIUS,
  COMET_HALO_STRENGTH,
  COMET_MS,
  FX_EDGE,
  FX_FIRST_FRAME_MS,
  PULSE_ALPHA,
  PULSE_MS,
  SWEEP_MS,
  WARP_MS,
} from "./tuning.ts";

/** Which effects play, and for how long. */
export interface CursorEffectSettings {
  readonly trail: TrailKind;
  readonly pulse: PulseKind;
  /** How long each lasts against its own length: 2 is twice as long. Left out, 1. */
  readonly length?: number | undefined;
  /** A plugin's shader drawn as the trail, in place of the built-in one. */
  readonly customTrail?: CustomEffect | undefined;
  /** A plugin's shader drawn as the pulse, in place of the built-in one. */
  readonly customPulse?: CustomEffect | undefined;
}

/** A plugin's effect that is playing: what it is, where, and since when. */
interface PlayingCustom {
  readonly effect: CustomEffect;
  readonly from: FxRect;
  readonly to: FxRect;
  readonly start: number;
  readonly duration: number;
  readonly color: number;
  /** Whether the cursor's cell is left for its pane to draw, as a trail leaves it. */
  readonly hole: boolean;
}

/** How long after a plugin's effect played the loss of WebGL is laid at its door. */
const CUSTOM_BLAME_MS = 2000;

/** Where the effects' canvas goes in the page. */
export interface FxSurface<Canvas> {
  createCanvas(): Canvas;
  show(canvas: Canvas): void;
  remove(canvas: Canvas): void;
  setCssSize(canvas: Canvas, size: Size): void;
  /** Hide the canvas, or show it again. A hidden one costs the page nothing. */
  setVisible(canvas: Canvas, visible: boolean): void;
}

/** Makes what draws the effects on a canvas: WebGL2 if it can, or Canvas 2D. */
export interface FxMakers<Canvas> {
  /** Throws when WebGL2 cannot be had. Left out, Canvas 2D is used. */
  readonly webgl2?: ((canvas: Canvas) => FxBackend) | undefined;
  readonly canvas2d: (canvas: Canvas) => FxBackend;
}

const NO_EFFECTS: CursorEffectSettings = { trail: "none", pulse: "none" };

const TRAIL_MS: Readonly<Record<Exclude<TrailKind, "none">, number>> = {
  warp: WARP_MS,
  sweep: SWEEP_MS,
  comet: COMET_MS,
};

const PULSE_SHAPES: Readonly<Record<Exclude<PulseKind, "none">, ShapeKind>> = {
  burst: ShapeKind.Disc,
  ripple: ShapeKind.Ring,
  "box-burst": ShapeKind.Box,
  "box-ripple": ShapeKind.BoxRing,
};

export class CursorEffects<Canvas> implements FrameView {
  readonly #surface: FxSurface<Canvas>;
  readonly #makers: FxMakers<Canvas>;
  readonly #requestFrame: () => void;
  readonly #onDispose: (() => void) | undefined;
  #settings: CursorEffectSettings = NO_EFFECTS;
  #canvas: Canvas | undefined;
  #backend: FxBackend | undefined;
  /** Whether WebGL2 has been given up for this window. */
  #webgl2Lost = false;
  #visible = false;
  #size: Size = { width: 0, height: 0 };
  #dpr = 1;
  #trail: Trail | undefined;
  #trailColor = 0;
  /** Where the trail under way has got to: where a trail that starts now leaves from. */
  #trailing: Corners | undefined;
  #pulse: Pulse | undefined;
  #pulseColor = 0;
  readonly #shapes = [new FxShape(), new FxShape()];
  readonly #played = { trails: 0, pulses: 0 };
  #disposed = false;
  #customTrail: PlayingCustom | undefined;
  #customPulse: PlayingCustom | undefined;
  /** The plugins' effects that could not be drawn with, which are not tried again. */
  readonly #failed = new Set<string>();
  /** When a plugin's effect last played. */
  #customAt = -Infinity;
  #problem: ((id: string, message: string) => void) | undefined;

  /**
   * `requestFrame` asks the window's loop for a frame, and `onDispose` is told when
   * the effects are given up.
   */
  constructor(
    surface: FxSurface<Canvas>,
    makers: FxMakers<Canvas>,
    requestFrame: () => void,
    onDispose?: () => void,
  ) {
    this.#surface = surface;
    this.#makers = makers;
    this.#requestFrame = requestFrame;
    this.#onDispose = onDispose;
  }

  /** What draws the effects: nothing while none is turned on. */
  get backend(): "webgl2" | "canvas2d" | undefined {
    return this.#backend?.kind;
  }

  /** How many trails and pulses have started, for measurements to check. */
  get played(): { readonly trails: number; readonly pulses: number } {
    return { ...this.#played };
  }

  /** Whether an effect is playing. */
  get playing(): boolean {
    return (
      this.#trail !== undefined ||
      this.#pulse !== undefined ||
      this.#customTrail !== undefined ||
      this.#customPulse !== undefined
    );
  }

  /** Told when a plugin's effect cannot be drawn: its id and why. It is not tried again. */
  onProblem(listener: (id: string, message: string) => void): void {
    this.#problem = listener;
  }

  /** Whether a plugin's effect can be drawn here, as far as is known. */
  #usable(effect: CustomEffect | undefined): effect is CustomEffect {
    return (
      effect !== undefined &&
      !this.#failed.has(effect.id) &&
      this.#backend?.prepareCustom !== undefined
    );
  }

  #fail(id: string, message: string): void {
    if (this.#failed.has(id)) return;
    this.#failed.add(id);
    this.#backend?.forgetCustom?.(id);
    this.#problem?.(id, message);
  }

  /**
   * Choose the effects. What draws them is made as soon as one is turned on, not when
   * the first plays, so that the first jump does not wait for a shader to compile; it
   * is given up when they are all turned off.
   */
  configure(settings: CursorEffectSettings): void {
    if (this.#disposed) return;
    this.#settings = settings;
    if (settings.trail === "none") this.#endTrail();
    if (settings.pulse === "none") this.#pulse = undefined;
    if (settings.customTrail?.id !== this.#customTrail?.effect.id) this.#customTrail = undefined;
    if (settings.customPulse?.id !== this.#customPulse?.effect.id) this.#customPulse = undefined;
    const wanted =
      settings.trail !== "none" ||
      settings.pulse !== "none" ||
      settings.customTrail !== undefined ||
      settings.customPulse !== undefined;
    if (wanted && !this.#backend) this.#start();
    if (!wanted && this.#backend) this.#stop();
    this.#prepareCustoms();
  }

  /** Builds the plugins' shaders now, so the first effect does not wait for them. */
  #prepareCustoms(): void {
    const backend = this.#backend;
    if (!backend?.prepareCustom) return;
    for (const effect of [this.#settings.customTrail, this.#settings.customPulse]) {
      if (!effect || this.#failed.has(effect.id)) continue;
      try {
        backend.prepareCustom(effect.id, effect.source);
      } catch (error) {
        this.#fail(effect.id, error instanceof Error ? error.message : String(error));
      }
    }
  }

  /** The window's size in CSS pixels, and its device pixel ratio. */
  resize(size: Size, dpr: number): void {
    this.#size = size;
    this.#dpr = dpr;
    if (this.#canvas !== undefined && this.#backend) this.#fit(this.#canvas, this.#backend);
  }

  /**
   * The cursor jumped from `from` to `to`, rectangles in CSS pixels of the window.
   * A trail plays if one is turned on and the jump is far enough; it leaves from where
   * a trail still under way has got to. Says whether one started.
   */
  move(from: FxRect, to: FxRect, color: number, now: number): boolean {
    const custom = this.#settings.customTrail;
    if (this.#usable(custom)) {
      if (!farEnough(cornersOf(from), to)) return false;
      this.#customTrail = {
        effect: custom,
        from,
        to,
        start: now - FX_FIRST_FRAME_MS,
        duration: custom.duration * this.#length(),
        color,
        hole: true,
      };
      this.#played.trails++;
      this.#requestFrame();
      return true;
    }
    const kind = this.#settings.trail;
    if (kind === "none" || !this.#backend) return false;
    // Whether the cursor itself went far, not where a trail under way has got to:
    // typing just after a jump must not start the trail again with each key.
    if (!farEnough(cornersOf(from), to)) {
      // A short step during a trail moves where the trail ends, not where it leaves.
      if (this.#trail) this.#retarget(to);
      return false;
    }
    const leaving = this.#trailing ?? cornersOf(from);
    const duration = TRAIL_MS[kind] * this.#length();
    this.#trail = startTrail(
      kind,
      leaving,
      to,
      now - FX_FIRST_FRAME_MS,
      duration,
      this.#size.height,
    );
    this.#trailing = leaving;
    this.#trailColor = color;
    this.#played.trails++;
    this.#requestFrame();
    return true;
  }

  /** The cursor changed shape, or landed in another pane, at `at`. */
  pulse(at: FxRect, color: number, now: number): boolean {
    const custom = this.#settings.customPulse;
    if (this.#usable(custom)) {
      this.#customPulse = {
        effect: custom,
        from: at,
        to: at,
        start: now - FX_FIRST_FRAME_MS,
        duration: custom.duration * this.#length(),
        color,
        hole: false,
      };
      this.#played.pulses++;
      this.#requestFrame();
      return true;
    }
    const kind = this.#settings.pulse;
    if (kind === "none" || !this.#backend) return false;
    this.#pulse = {
      kind,
      start: now - FX_FIRST_FRAME_MS,
      duration: PULSE_MS * this.#length(),
      at,
    };
    this.#pulseColor = color;
    this.#played.pulses++;
    this.#requestFrame();
    return true;
  }

  /** Stop whatever is playing, at once. */
  cancel(): void {
    this.#endTrail();
    this.#pulse = undefined;
    this.#customTrail = undefined;
    this.#customPulse = undefined;
    this.#hide();
  }

  needsFrame(): boolean {
    return this.playing;
  }

  frame(now: number): void {
    const backend = this.#backend;
    if (!backend || this.#canvas === undefined) return;
    let count = 0;
    if (this.#trail) {
      const shape = this.#shapes[count] as FxShape;
      if (this.#fillTrail(shape, this.#trail, now)) count++;
    }
    if (this.#pulse) {
      const shape = this.#shapes[count] as FxShape;
      if (this.#fillPulse(shape, this.#pulse, now)) count++;
    }
    const customs = this.#playingCustoms(now);
    if (count === 0 && customs.length === 0) {
      this.#hide();
      return;
    }
    backend.draw(this.#shapes, count);
    for (const custom of customs) this.#drawCustom(backend, custom, now);
    if (!this.#visible) {
      this.#visible = true;
      this.#surface.setVisible(this.#canvas, true);
    }
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#stop();
    this.#onDispose?.();
  }

  /** How long effects last against their own length. */
  #length(): number {
    const length = this.#settings.length;
    return length !== undefined && Number.isFinite(length) && length > 0 ? length : 1;
  }

  /** The plugins' effects still playing at `now`. Those that are over end. */
  #playingCustoms(now: number): PlayingCustom[] {
    const playing: PlayingCustom[] = [];
    for (const which of ["#customTrail", "#customPulse"] as const) {
      const custom = which === "#customTrail" ? this.#customTrail : this.#customPulse;
      if (!custom) continue;
      if (now - custom.start >= custom.duration || this.#failed.has(custom.effect.id)) {
        if (which === "#customTrail") this.#customTrail = undefined;
        else this.#customPulse = undefined;
        continue;
      }
      playing.push(custom);
    }
    return playing;
  }

  #drawCustom(backend: FxBackend, custom: PlayingCustom, now: number): void {
    const dpr = this.#dpr;
    const rect = (r: FxRect): [number, number, number, number] => [
      r.x * dpr,
      r.y * dpr,
      r.width * dpr,
      r.height * dpr,
    ];
    const reach = custom.effect.reach * dpr;
    const left = Math.min(custom.from.x, custom.to.x) * dpr - reach;
    const top = Math.min(custom.from.y, custom.to.y) * dpr - reach;
    const right =
      Math.max(custom.from.x + custom.from.width, custom.to.x + custom.to.width) * dpr + reach;
    const bottom =
      Math.max(custom.from.y + custom.from.height, custom.to.y + custom.to.height) * dpr + reach;
    const width = Math.max(1, Math.round(this.#size.width * dpr));
    const height = Math.max(1, Math.round(this.#size.height * dpr));
    const to = custom.to;
    try {
      this.#customAt = now;
      backend.drawCustom?.(custom.effect.id, {
        progress: Math.min(1, Math.max(0, (now - custom.start) / custom.duration)),
        time: Math.max(0, now - custom.start) / 1000,
        from: rect(custom.from),
        to: rect(custom.to),
        color: custom.color,
        bounds: [
          Math.max(0, Math.floor(left)),
          Math.max(0, Math.floor(top)),
          Math.min(width, Math.ceil(right)),
          Math.min(height, Math.ceil(bottom)),
        ],
        hole: custom.hole
          ? [
              Math.round(to.x * dpr) - FX_EDGE,
              Math.round(to.y * dpr) - FX_EDGE,
              Math.round((to.x + to.width) * dpr) + FX_EDGE,
              Math.round((to.y + to.height) * dpr) + FX_EDGE,
            ]
          : [0, 0, 0, 0],
      });
    } catch (error) {
      if (!(error instanceof CustomEffectError)) throw error;
      this.#fail(custom.effect.id, error.message);
    }
  }

  #start(): void {
    const [canvas, backend] = this.#make();
    this.#canvas = canvas;
    this.#backend = backend;
    this.#visible = false;
    this.#surface.setVisible(canvas, false);
    this.#fit(canvas, backend);
    this.#surface.show(canvas);
  }

  #make(): [Canvas, FxBackend] {
    const webgl2 = this.#makers.webgl2;
    if (webgl2 && !this.#webgl2Lost) {
      const canvas = this.#surface.createCanvas();
      try {
        const backend = webgl2(canvas);
        backend.onLost(() => this.#replace());
        return [canvas, backend];
      } catch {
        // Canvas 2D does, on a canvas of its own: that one may already be WebGL's.
        this.#webgl2Lost = true;
      }
    }
    const canvas = this.#surface.createCanvas();
    return [canvas, this.#makers.canvas2d(canvas)];
  }

  /** WebGL2 was lost: go on with Canvas 2D on a new canvas, playing what was playing. */
  #replace(): void {
    if (this.#disposed) return;
    // A plugin's shader that played just before is taken to have cost the window it.
    if (performance.now() - this.#customAt < CUSTOM_BLAME_MS) {
      for (const custom of [this.#settings.customTrail, this.#settings.customPulse]) {
        if (custom) this.#fail(custom.id, "the graphics card stopped while it drew");
      }
    }
    this.#customTrail = undefined;
    this.#customPulse = undefined;
    this.#webgl2Lost = true;
    this.#stop();
    if (this.#settings.trail !== "none" || this.#settings.pulse !== "none") this.#start();
    if (this.playing) this.#requestFrame();
  }

  #stop(): void {
    this.#endTrail();
    this.#pulse = undefined;
    this.#customTrail = undefined;
    this.#customPulse = undefined;
    this.#backend?.dispose();
    this.#backend = undefined;
    if (this.#canvas !== undefined) this.#surface.remove(this.#canvas);
    this.#canvas = undefined;
    this.#visible = false;
  }

  #fit(canvas: Canvas, backend: FxBackend): void {
    const { width, height } = this.#size;
    this.#surface.setCssSize(canvas, this.#size);
    backend.resize(
      Math.max(1, Math.round(width * this.#dpr)),
      Math.max(1, Math.round(height * this.#dpr)),
    );
  }

  #hide(): void {
    if (!this.#visible || this.#canvas === undefined) return;
    this.#backend?.clear();
    this.#visible = false;
    this.#surface.setVisible(this.#canvas, false);
  }

  #endTrail(): void {
    this.#trail = undefined;
    this.#trailing = undefined;
  }

  /**
   * A short step while a trail plays, as when typing starts at once after a jump: the
   * trail keeps its start and its time, and now ends where the cursor is.
   */
  #retarget(to: FxRect): void {
    const trail = this.#trail;
    if (!trail) return;
    this.#trail = startTrail(
      trail.kind,
      trail.from,
      to,
      trail.start,
      trail.duration,
      this.#size.height,
    );
  }

  /** Fills `shape` with the trail at `now`. Says whether there is anything to draw. */
  #fillTrail(shape: FxShape, trail: Trail, now: number): boolean {
    const outline = trailShape(trail, now, shape.points);
    if (outline.done) {
      this.#endTrail();
      return false;
    }
    this.#trailing = outline.trailing;
    const dpr = this.#dpr;
    const comet = trail.kind === "comet";
    shape.kind = ShapeKind.Polygon;
    shape.count = outline.count;
    let left = Infinity;
    let top = Infinity;
    let right = -Infinity;
    let bottom = -Infinity;
    for (let index = 0; index < outline.count * 2; index += 2) {
      const x = (shape.points[index] ?? 0) * dpr;
      const y = (shape.points[index + 1] ?? 0) * dpr;
      shape.points[index] = x;
      shape.points[index + 1] = y;
      left = Math.min(left, x);
      top = Math.min(top, y);
      right = Math.max(right, x);
      bottom = Math.max(bottom, y);
    }
    shape.color = this.#trailColor;
    shape.alpha = 1;
    shape.fadePower = comet ? COMET_FADE_POWER : 0;
    shape.fadeFromX = outline.tail.x * dpr;
    shape.fadeFromY = outline.tail.y * dpr;
    shape.fadeToX = outline.head.x * dpr;
    shape.fadeToY = outline.head.y * dpr;
    shape.haloRadius = comet ? COMET_HALO_RADIUS * dpr : 0;
    shape.haloStrength = comet ? COMET_HALO_STRENGTH : 0;
    // The cursor itself is drawn by its pane: the trail leaves its cell alone, and the
    // smoothed edge round it too, which would otherwise ring the cell with a fringe.
    shape.holeLeft = Math.round(trail.to.x * dpr) - FX_EDGE;
    shape.holeTop = Math.round(trail.to.y * dpr) - FX_EDGE;
    shape.holeRight = Math.round((trail.to.x + trail.to.width) * dpr) + FX_EDGE;
    shape.holeBottom = Math.round((trail.to.y + trail.to.height) * dpr) + FX_EDGE;
    this.#bound(shape, left, top, right, bottom);
    return true;
  }

  #fillPulse(shape: FxShape, pulse: Pulse, now: number): boolean {
    const grown = pulseShape(pulse, now);
    if (grown.done) {
      this.#pulse = undefined;
      return false;
    }
    const dpr = this.#dpr;
    shape.kind = PULSE_SHAPES[pulse.kind];
    shape.count = 0;
    shape.centreX = grown.centre.x * dpr;
    shape.centreY = grown.centre.y * dpr;
    shape.radius = grown.radius * dpr;
    shape.halfWidth = grown.halfWidth * dpr;
    shape.halfHeight = grown.halfHeight * dpr;
    shape.ring = grown.ring * dpr;
    shape.color = this.#pulseColor;
    shape.alpha = PULSE_ALPHA * grown.fade;
    shape.fadePower = 0;
    shape.haloRadius = 0;
    shape.haloStrength = 0;
    shape.holeLeft = 0;
    shape.holeTop = 0;
    shape.holeRight = 0;
    shape.holeBottom = 0;
    const round = shape.kind === ShapeKind.Disc || shape.kind === ShapeKind.Ring;
    const reachX = (round ? shape.radius : shape.halfWidth) + shape.ring / 2;
    const reachY = (round ? shape.radius : shape.halfHeight) + shape.ring / 2;
    this.#bound(
      shape,
      shape.centreX - reachX,
      shape.centreY - reachY,
      shape.centreX + reachX,
      shape.centreY + reachY,
    );
    return true;
  }

  /** Sets what a shape lies within: its own extent, its smoothed edge and its glow. */
  #bound(shape: FxShape, left: number, top: number, right: number, bottom: number): void {
    // A glow is all but gone three of its radii out.
    const pad = FX_EDGE + 1 + shape.haloRadius * 3;
    shape.left = Math.max(0, Math.floor(left - pad));
    shape.top = Math.max(0, Math.floor(top - pad));
    shape.right = Math.ceil(right + pad);
    shape.bottom = Math.ceil(bottom + pad);
  }
}
