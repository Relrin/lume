/**
 * Turning shapes into coverage masks without a canvas.
 *
 * Box-drawing characters, block elements and underlines are drawn from shapes instead of
 * from the font, so that they fill their cell exactly and meet their neighbours with no
 * gap. Doing it in plain arithmetic, not with a 2D context, makes the result the same in
 * every webview and lets it be checked pixel by pixel in tests.
 *
 * Rectangles are placed on whole pixels by whoever builds them, so they cover each pixel
 * fully or not at all. Curves and diagonals are sampled several times per pixel.
 */

import { PROCEDURAL_SUPERSAMPLE } from "../constants.ts";

/** A shape in pixel coordinates within a mask, with the origin at the top-left corner. */
export type Shape =
  /** A filled box from `x0`, `y0` up to but not including `x1`, `y1`. */
  | { kind: "rect"; x0: number; y0: number; x1: number; y1: number; alpha?: number }
  /** A straight stroke of the given width with square ends. */
  | { kind: "line"; x0: number; y0: number; x1: number; y1: number; width: number }
  /**
   * A quarter-circle stroke around `cx`, `cy` with radius `r`. `quadrant` picks the
   * quarter by the signs of x and y from the centre: 0 is right and down, 1 left and
   * down, 2 left and up, 3 right and up.
   */
  | { kind: "arc"; cx: number; cy: number; r: number; quadrant: 0 | 1 | 2 | 3; width: number };

/** A coverage mask: one byte per pixel, rows from the top, 255 fully covered. */
export interface Mask {
  width: number;
  height: number;
  pixels: Uint8Array;
}

/** Draw shapes into a new mask. Where shapes overlap, the greater coverage wins. */
export function rasterise(shapes: readonly Shape[], width: number, height: number): Mask {
  const pixels = new Uint8Array(width * height);
  for (const shape of shapes) {
    if (shape.kind === "rect") fillRect(pixels, width, height, shape);
    else sample(pixels, width, height, shape);
  }
  return { width, height, pixels };
}

function fillRect(
  pixels: Uint8Array,
  width: number,
  height: number,
  rect: Extract<Shape, { kind: "rect" }>,
): void {
  const x0 = Math.max(0, Math.round(rect.x0));
  const y0 = Math.max(0, Math.round(rect.y0));
  const x1 = Math.min(width, Math.round(rect.x1));
  const y1 = Math.min(height, Math.round(rect.y1));
  const value = Math.round(255 * (rect.alpha ?? 1));
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const at = y * width + x;
      if (pixels[at]! < value) pixels[at] = value;
    }
  }
}

/** Whether a point is inside a stroke. */
function inside(shape: Exclude<Shape, { kind: "rect" }>, x: number, y: number): boolean {
  const half = shape.width / 2;
  if (shape.kind === "line") {
    const dx = shape.x1 - shape.x0;
    const dy = shape.y1 - shape.y0;
    const length = Math.hypot(dx, dy);
    if (length === 0) return false;
    // Distance along the stroke, and distance from its centre line.
    const along = ((x - shape.x0) * dx + (y - shape.y0) * dy) / length;
    const across = Math.abs((x - shape.x0) * dy - (y - shape.y0) * dx) / length;
    return along >= 0 && along <= length && across <= half;
  }
  const dx = x - shape.cx;
  const dy = y - shape.cy;
  const right = shape.quadrant === 0 || shape.quadrant === 3;
  const down = shape.quadrant === 0 || shape.quadrant === 1;
  if (right ? dx < 0 : dx > 0) return false;
  if (down ? dy < 0 : dy > 0) return false;
  return Math.abs(Math.hypot(dx, dy) - shape.r) <= half;
}

function sample(
  pixels: Uint8Array,
  width: number,
  height: number,
  shape: Exclude<Shape, { kind: "rect" }>,
): void {
  const n = PROCEDURAL_SUPERSAMPLE;
  const total = n * n;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      let hits = 0;
      for (let sy = 0; sy < n; sy++) {
        for (let sx = 0; sx < n; sx++) {
          if (inside(shape, x + (sx + 0.5) / n, y + (sy + 0.5) / n)) hits++;
        }
      }
      if (hits === 0) continue;
      const value = Math.round((255 * hits) / total);
      const at = y * width + x;
      if (pixels[at]! < value) pixels[at] = value;
    }
  }
}
