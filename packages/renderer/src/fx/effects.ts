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
}

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
    return this.#trail !== undefined || this.#pulse !== undefined;
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
    const wanted = settings.trail !== "none" || settings.pulse !== "none";
    if (wanted && !this.#backend) this.#start();
    if (!wanted && this.#backend) this.#stop();
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
    if (count === 0) {
      this.#hide();
      return;
    }
    backend.draw(this.#shapes, count);
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
    this.#webgl2Lost = true;
    this.#stop();
    if (this.#settings.trail !== "none" || this.#settings.pulse !== "none") this.#start();
    if (this.playing) this.#requestFrame();
  }

  #stop(): void {
    this.#endTrail();
    this.#pulse = undefined;
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
