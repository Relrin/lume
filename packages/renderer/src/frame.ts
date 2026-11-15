/**
 * The frame the renderer draws.
 *
 * A frame is one reusable object holding the whole viewport as parallel typed arrays, one
 * entry per cell in row-major order, with colours already resolved. There are no per-cell
 * objects: a 200 by 60 grid is twelve thousand cells and is redrawn many times a second.
 *
 * Whatever produces frames declares the same shape and the same numbers for itself, so
 * that neither side depends on the other. The two declarations are compared where both
 * are visible.
 *
 * This file imports nothing.
 */

/** How much of a frame changed since it was last marked drawn. */
export const DirtyState = {
  /** Nothing changed. */
  None: 0,
  /** The rows flagged in `dirtyRows` changed, or only the cursor did. */
  Partial: 1,
  /** Everything must be redrawn. */
  Full: 2,
} as const;

/** Bits of a cell's entry in `flags`. */
export const CellFlag = {
  Bold: 1 << 0,
  Italic: 1 << 1,
  Strikethrough: 1 << 2,
  Overline: 1 << 3,
  /** The first cell of a double-width character. */
  Wide: 1 << 7,
  /** The second cell of a double-width character. It has no glyph of its own. */
  Spacer: 1 << 8,
  /** The cell is part of a hyperlink. */
  Hyperlink: 1 << 9,
} as const;

/** The underline style occupies three bits of `flags`, read as `(flags >> 4) & 7`. */
export const UNDERLINE_SHIFT = 4;
export const UNDERLINE_MASK = 0x7;

/** Values of the underline style bits. */
export const UnderlineStyle = {
  None: 0,
  Single: 1,
  Double: 2,
  Curly: 3,
  Dotted: 4,
  Dashed: 5,
} as const;

/** The shape the cursor is drawn in. */
export const CursorShape = {
  Bar: 0,
  Block: 1,
  Underline: 2,
  HollowBlock: 3,
} as const;

/**
 * A `glyph` entry with this bit set is not a codepoint: its low bits index `clusters`,
 * the text of cells that hold more than one codepoint.
 */
export const GLYPH_CLUSTER_TAG = 0x8000_0000;

/** A `selectionStart` entry for a row with no selected cells. */
export const NO_SELECTION = -1;

/** The cursor as it should be drawn. */
export interface FrameCursor {
  /** Whether to draw it at all: shown by the program and inside the viewport. */
  visible: boolean;
  x: number;
  y: number;
  /** A `CursorShape`. */
  shape: number;
  /** Whether the program asked for a blinking cursor. Blinking is the drawer's job. */
  blinking: boolean;
  /** The cursor sits on the second cell of a double-width character. */
  wideTail: boolean;
  /** Whether the program set a cursor colour. */
  hasColor: boolean;
  /** That colour, when `hasColor`. */
  color: number;
}

/**
 * One viewport of cells, ready to draw.
 *
 * Colours are four bytes R, G, B, A in memory order, so on the little-endian machines
 * this runs on the number is `r | g << 8 | b << 16 | a << 24`. A background with alpha 0
 * means "the default background". Foregrounds are always opaque.
 *
 * The arrays always describe the complete viewport, not only the rows that changed.
 */
export interface RenderFrame {
  cols: number;
  rows: number;
  /** A `DirtyState`. It accumulates until the frame is marked drawn. */
  dirty: number;
  /** One entry per row: 1 when the row changed. */
  dirtyRows: Uint8Array;

  /** Per cell: 0 for empty, a codepoint, or `GLYPH_CLUSTER_TAG | index`. */
  glyph: Uint32Array;
  /** Per cell: the colour of the text. */
  fg: Uint32Array;
  /** Per cell: the background, or alpha 0 for the default background. */
  bg: Uint32Array;
  /** Per cell: the colour of an underline, which a program may set apart from the text. */
  underlineColor: Uint32Array;
  /** Per cell: `CellFlag` bits and the underline style. */
  flags: Uint16Array;

  /** Per row: the first selected column, or `NO_SELECTION`. */
  selectionStart: Int32Array;
  /** Per row: the last selected column, inclusive. Meaningful when the row has a selection. */
  selectionEnd: Int32Array;

  /** The text of multi-codepoint cells, indexed by tagged `glyph` entries. */
  clusters: readonly string[];

  cursor: FrameCursor;
  /** The default text colour. */
  foreground: number;
  /** The default background colour. */
  background: number;
}

/** Pack a colour as `RenderFrame` stores it. */
export function rgba(r: number, g: number, b: number, a = 255): number {
  return (r | (g << 8) | (b << 16) | (a << 24)) >>> 0;
}
