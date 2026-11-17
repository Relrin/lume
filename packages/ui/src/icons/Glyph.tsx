/**
 * The page's drawings: line drawings in the colour of the text around them, and the
 * marks of programs filled in it.
 *
 * Each is decoration beside a label or inside a button that names itself, so none is
 * announced on its own.
 */

import { createElement } from "react";

import {
  BRAND_GLYPHS,
  LINE_GLYPHS,
  type BrandGlyphName,
  type LineGlyphName,
} from "./glyphs.generated.ts";

/**
 * A box a drawing fills, its drawing `size` pixels square in its middle.
 *
 * The browser puts an element's box on whole pixels, so a drawing placed by its own
 * box half a pixel from its neighbour's edge is moved by that half pixel. A drawing
 * that fills the box it is centred in is drawn where it should be at any pixel ratio.
 */
export interface Frame {
  readonly width: number;
  readonly height: number;
}

/** The svg attributes that draw a 24-unit square `size` pixels across, centred in `frame`. */
function placed(size: number, frame: Frame | undefined) {
  if (!frame) return { width: size, height: size, viewBox: "0 0 24 24" };
  const units = 24 / size;
  const width = frame.width * units;
  const height = frame.height * units;
  const left = 12 - width / 2;
  const top = 12 - height / 2;
  return { width: frame.width, height: frame.height, viewBox: `${left} ${top} ${width} ${height}` };
}

/**
 * A line drawing, `size` pixels square, its lines `weight` units wide out of 24, in the
 * middle of `frame` when it is given one to fill.
 */
export function Glyph({
  name,
  size,
  weight = 2,
  frame,
}: {
  name: LineGlyphName;
  size: number;
  weight?: number;
  frame?: Frame;
}) {
  return (
    <svg
      {...placed(size, frame)}
      fill="none"
      stroke="currentColor"
      strokeWidth={weight}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {LINE_GLYPHS[name].map(([tag, attributes], index) =>
        createElement(tag, { key: index, ...attributes }),
      )}
    </svg>
  );
}

/** The mark of a program, `size` pixels square, in the middle of `frame` when given one. */
export function BrandMark({
  name,
  size,
  frame,
}: {
  name: BrandGlyphName;
  size: number;
  frame?: Frame;
}) {
  return (
    <svg {...placed(size, frame)} fill="currentColor" aria-hidden="true">
      <path d={BRAND_GLYPHS[name].path} />
    </svg>
  );
}
