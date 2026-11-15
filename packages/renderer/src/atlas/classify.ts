/**
 * Turning the pixels of a freshly drawn glyph into what the atlas stores.
 *
 * A glyph is drawn in white on a transparent canvas. Two things are worked out from the
 * result: the box that actually has ink in it, so that only that is stored, and whether
 * the glyph is a plain shape or brought colours of its own.
 *
 * The test for colour is whether any inked pixel is not white. Text comes back white
 * however faint its edges are, because antialiasing changes alpha and not colour. An
 * emoji comes back in its own colours, including black and grey ones, which a test for
 * "not grey" would get wrong.
 */

import { CLASSIFY_MIN_ALPHA, CLASSIFY_WHITE_TOLERANCE } from "../constants.ts";
import type { PageKind } from "../types.ts";

/** The inked part of a drawn glyph. */
export interface Ink {
  /** The inked box within the source image. A width of zero means no ink at all. */
  x: number;
  y: number;
  width: number;
  height: number;
  kind: PageKind;
  /** The pixels of the box: coverage for a mask, R, G, B, A for colour. */
  pixels: Uint8Array;
}

/**
 * Find the ink in an image of `width` by `height` pixels given as R, G, B, A bytes that
 * are not premultiplied.
 */
export function classify(rgba: Uint8ClampedArray | Uint8Array, width: number, height: number): Ink {
  let left = width;
  let right = -1;
  let top = height;
  let bottom = -1;
  let color = false;
  const white = 255 - CLASSIFY_WHITE_TOLERANCE;

  for (let y = 0; y < height; y++) {
    const row = y * width * 4;
    for (let x = 0; x < width; x++) {
      const at = row + x * 4;
      const alpha = rgba[at + 3]!;
      if (alpha === 0) continue;
      if (x < left) left = x;
      if (x > right) right = x;
      if (y < top) top = y;
      if (y > bottom) bottom = y;
      // Nearly transparent pixels carry too little colour to judge by.
      if (
        !color &&
        alpha >= CLASSIFY_MIN_ALPHA &&
        (rgba[at]! < white || rgba[at + 1]! < white || rgba[at + 2]! < white)
      ) {
        color = true;
      }
    }
  }

  if (right < 0) {
    return { x: 0, y: 0, width: 0, height: 0, kind: "mask", pixels: new Uint8Array(0) };
  }

  const boxWidth = right - left + 1;
  const boxHeight = bottom - top + 1;
  if (color) {
    const pixels = new Uint8Array(boxWidth * boxHeight * 4);
    for (let y = 0; y < boxHeight; y++) {
      const from = ((top + y) * width + left) * 4;
      pixels.set(rgba.subarray(from, from + boxWidth * 4), y * boxWidth * 4);
    }
    return { x: left, y: top, width: boxWidth, height: boxHeight, kind: "color", pixels };
  }

  const pixels = new Uint8Array(boxWidth * boxHeight);
  for (let y = 0; y < boxHeight; y++) {
    const from = ((top + y) * width + left) * 4 + 3;
    for (let x = 0; x < boxWidth; x++) pixels[y * boxWidth + x] = rgba[from + x * 4]!;
  }
  return { x: left, y: top, width: boxWidth, height: boxHeight, kind: "mask", pixels };
}
