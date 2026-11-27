/**
 * One shape an effect draws in a frame, in device pixels, and what draws it.
 *
 * A shape is a record of numbers that is filled in again every frame rather than made
 * anew, so drawing an effect makes nothing for the garbage collector.
 */

import type { CustomDraw } from "./custom.ts";
import { FX_POLYGON_MAX } from "./tuning.ts";

/** What a shape is: a trail's outline, or one of the pulses. */
export const ShapeKind = {
  Polygon: 0,
  Disc: 1,
  Ring: 2,
  Box: 3,
  BoxRing: 4,
} as const;
export type ShapeKind = (typeof ShapeKind)[keyof typeof ShapeKind];

/** A shape to draw. Every length and place is in device pixels from the top left. */
export class FxShape {
  kind: ShapeKind = ShapeKind.Polygon;
  /** A polygon's corners as x, y pairs, going round one way. */
  readonly points = new Float32Array(FX_POLYGON_MAX * 2);
  count = 0;
  /** A disc's, ring's or box's middle; a disc's radius; a box's half sizes. */
  centreX = 0;
  centreY = 0;
  radius = 0;
  halfWidth = 0;
  halfHeight = 0;
  /** How thick a ring or a box's outline is. */
  ring = 0;
  /** Packed as a frame packs colours. Its own alpha is not used: `alpha` is. */
  color = 0;
  alpha = 1;
  /** A fade from nothing at `fadeFrom` to all at `fadeTo`, raised to `fadePower`; 0 for none. */
  fadeFromX = 0;
  fadeFromY = 0;
  fadeToX = 0;
  fadeToY = 0;
  fadePower = 0;
  /** A glow round the shape: how far it reaches, and how bright it starts; 0 for none. */
  haloRadius = 0;
  haloStrength = 0;
  /** A rectangle left clear, where the cursor itself is: left, top, right, bottom. */
  holeLeft = 0;
  holeTop = 0;
  holeRight = 0;
  holeBottom = 0;
  /** The rectangle the shape and its glow lie within: left, top, right, bottom. */
  left = 0;
  top = 0;
  right = 0;
  bottom = 0;
}

/** What draws the effects onto their canvas. */
export interface FxBackend {
  readonly kind: "webgl2" | "canvas2d";
  /** Make the canvas `width` by `height` device pixels. */
  resize(width: number, height: number): void;
  /** Draw the first `count` shapes, over nothing. */
  draw(shapes: readonly FxShape[], count: number): void;
  /** Leave the canvas empty. */
  clear(): void;
  /** Told once if the backend can no longer draw. */
  onLost(listener: () => void): void;
  dispose(): void;
  /**
   * Whether `draw` draws now: false while the backend's own program is still being
   * built beside the page, without waiting for it. A backend without one is ready.
   */
  ready?(): boolean;
  /**
   * Begins building a plugin's shader to be drawn with. Only WebGL2 has these; with
   * Canvas 2D a plugin's effect is not drawn. The building goes on beside the page
   * where the browser can do that, and `customReady` says when it is over.
   */
  prepareCustom?(id: string, source: string): void;
  /**
   * Whether a plugin's shader is built and can be drawn with: false while it is
   * still being built, without waiting for it. Throws a `CustomEffectError` when it
   * could not be built.
   */
  customReady?(id: string): boolean;
  /**
   * Draws a frame of a plugin's effect over what `draw` drew. A shader `customReady`
   * has not said is built draws nothing: a frame never waits for one.
   */
  drawCustom?(id: string, frame: CustomDraw): void;
  /** Lets go of a plugin's shader. */
  forgetCustom?(id: string): void;
}

/** The red, green and blue of a packed colour, from 0 to 1. */
export function channels(color: number): [number, number, number] {
  return [(color & 0xff) / 255, ((color >>> 8) & 0xff) / 255, ((color >>> 16) & 0xff) / 255];
}
