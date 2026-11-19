/**
 * The types the renderer shares between its parts and with its callers.
 */

import type { GlyphAtlas } from "./atlas/atlas.ts";
import type { Highlights, RenderFrame } from "./frame.ts";
import type { PlaneName, Planes } from "./layout.ts";

/** A way of drawing: the best one a webview supports is used. */
export type Tier = "webgpu" | "webgl2" | "canvas2d";

/** A width and a height. */
export interface Size {
  width: number;
  height: number;
}

/** The font text is drawn in. */
export interface FontSpec {
  /** A CSS font-family list, most preferred first. */
  family: string;
  /** Size in CSS pixels. */
  sizePx: number;
  /** Line height as a multiple of the font's ascent plus descent. */
  lineHeight?: number;
  /** CSS weights for normal and bold text. */
  weight?: number;
  boldWeight?: number;
}

/**
 * How a terminal looks, apart from what the program in it decides. Colours are packed as
 * a `RenderFrame` packs them.
 */
export interface TerminalTheme {
  font: FontSpec;
  /** The cursor, unless the program set a cursor colour of its own. */
  cursor: number;
  /** Text under a block cursor. Without it the default background is used. */
  cursorText?: number;
  selectionBackground: number;
  /** Text inside the selection. Without it selected text keeps its colour. */
  selectionForeground?: number;
  /** Behind text a search found. It may be translucent: the cell's own colour shows through. */
  searchMatch: number;
  /** Behind the match a search is at. */
  searchActive: number;
  /** Text of that match. Without it the text keeps its colour. */
  searchActiveText?: number;
  /** Space between the edge of the surface and the grid, in CSS pixels. */
  padding: number;
}

/** A run of cells in one row of the grid. `endCol` is the last cell of the run. */
export interface UnderlineRange {
  row: number;
  startCol: number;
  endCol: number;
}

/** What a font measures, in device pixels, before anything is rounded. */
export interface RawFontMetrics {
  /** The advance of one character of the monospace font. */
  advance: number;
  ascent: number;
  descent: number;
}

/** The geometry of one cell, in whole device pixels. */
export interface CellMetrics {
  cellWidth: number;
  cellHeight: number;
  /** From the top of the cell to the text baseline. */
  baseline: number;
  /** Thickness of underlines, strikethrough and light box-drawing lines. */
  lineThickness: number;
  /** From the top of the cell to the top of a single underline. */
  underlineTop: number;
  /** From the top of the cell to the top of a strikethrough. */
  strikethroughTop: number;
  /** From the edge of the surface to the grid, the least there is on every side. */
  padding: number;
  /**
   * From the left and top edges of the surface to the grid. The padding, unless the grid
   * is drawn centred on a surface it does not fill.
   */
  originX: number;
  originY: number;
}

/**
 * Draws terminal frames onto a surface. Obtained from a `RenderHost`; the three tiers
 * differ only in what they draw with.
 */
export interface CellRenderer {
  /** The tier in use. It changes if the tier is lost and the next one takes over. */
  readonly tier: Tier;
  /** The cell geometry for the current font, device pixel ratio and theme. */
  readonly metrics: CellMetrics;
  /** Set the size of the surface in CSS pixels and the device pixel ratio. */
  resize(px: Size, dpr: number): void;
  setTheme(theme: TerminalTheme): void;
  /** Whether the terminal has keyboard focus. Without it the cursor is drawn hollow. */
  setFocused(focused: boolean): void;
  /**
   * Whether the cursor is drawn when the frame says it is visible. A blinking cursor is
   * the caller turning this off and on; the renderer keeps no time of its own.
   */
  setCursorVisible(visible: boolean): void;
  /**
   * Underline these runs of cells on top of whatever the frame draws there, in each
   * run's text colour: the link the pointer is over. Replaces the runs set before.
   */
  setUnderlines(ranges: readonly UnderlineRange[]): void;
  /**
   * Highlight these runs of cells: what a search found. Replaces the runs set before;
   * `undefined` or an empty list removes them. The list is copied, so the caller may
   * refill it. A selection is drawn over a highlight.
   */
  setHighlights(highlights: Highlights | undefined): void;
  /** The grid that fits the surface at its current size. */
  gridSize(): { cols: number; rows: number };
  /**
   * Whether a grid smaller than the surface is drawn in its middle rather than from the
   * top left: for a terminal whose size another view decides. A grid larger than the
   * surface is drawn from the top left either way, and what does not fit is cut off.
   */
  setCentred(centred: boolean): void;
  /** Where a grid of `cols` by `rows` is drawn on the surface now, in device pixels. */
  origin(cols: number, rows: number): { x: number; y: number };
  /** Draw a frame. Rows the frame does not flag as changed are not rebuilt. */
  draw(frame: RenderFrame): void;
  dispose(): void;
}

/** The kind of an atlas page: coverage to be tinted, or pixels with their own colour. */
export type PageKind = "mask" | "color";

/**
 * Where an atlas keeps its pixels. Each tier has its own: textures on a GPU, canvases
 * otherwise.
 */
export interface AtlasSink {
  /** Make sure `count` pages of a kind exist. Existing pages keep their contents. */
  ensurePages(kind: PageKind, count: number): void;
  /**
   * Store a glyph. `pixels` is one byte of coverage per pixel for a mask, and four bytes
   * (R, G, B, A, not premultiplied) per pixel for colour, in rows from the top.
   */
  upload(
    kind: PageKind,
    page: number,
    x: number,
    y: number,
    width: number,
    height: number,
    pixels: Uint8Array,
  ): void;
}

/** A glyph turned into pixels. */
export interface RasterGlyph {
  /** The size of the inked area. Zero width means the glyph draws nothing. */
  width: number;
  height: number;
  /** From the top-left corner of the glyph's first cell to the inked area. */
  offsetX: number;
  offsetY: number;
  kind: PageKind;
  pixels: Uint8Array;
}

/** Turns text into pixels. The real one uses a canvas; tests use a fake. */
export interface GlyphRasteriser {
  rasterise(text: string, bold: boolean, italic: boolean, span: number): RasterGlyph;
}

/** The font services a renderer needs from its surroundings. */
export interface FontBackend {
  /** Measure a font at a device pixel ratio. */
  measure(font: FontSpec, dpr: number): RawFontMetrics;
  /** A rasteriser for a font at a device pixel ratio and cell geometry. */
  rasteriser(font: FontSpec, dpr: number, metrics: CellMetrics): GlyphRasteriser;
}

/** What a tier needs to put one frame on its surface. */
export interface PresentInfo {
  /** The surface in device pixels. */
  width: number;
  height: number;
  /** The colour behind everything: the terminal's default background. */
  clear: number;
  planes: Planes;
  cols: number;
  rows: number;
  metrics: CellMetrics;
  /**
   * Rows whose pixels may differ from the last frame, one entry per row, or `undefined`
   * when the whole surface must be drawn. A tier whose surface keeps its contents
   * redraws only these; the others redraw everything from their buffers regardless.
   */
  repaint: Uint8Array | undefined;
  /** How many records of the overlay planes are in use. */
  overlayRects: number;
  overlayGlyphs: number;
  /**
   * How many records from the first of each plane laid out by cell may be other than
   * empty: the records of the rows down to the last that has any. A tier draws no more
   * than these; the records after them are all empty.
   */
  used: Readonly<Record<"background" | "underline" | "glyph" | "line", number>>;
}

/**
 * The part of a tier that touches its drawing interface. Everything else about drawing a
 * terminal is shared and sits above this, so a tier is only: keep the atlas pixels, keep
 * a copy of the instance buffers, and draw them.
 */
export interface TierBackend {
  readonly tier: Tier;
  /**
   * The atlas for glyphs of one font at one size. `create` makes a new one over the
   * tier's own store; a tier that can share glyph textures between panes hands back the
   * atlas it already has for the key.
   */
  atlas(key: string, create: (sink: AtlasSink) => GlyphAtlas): GlyphAtlas;
  /** Set the size of the surface in device pixels. */
  configure(width: number, height: number): void;
  /** Use these planes from now on. Called when the grid changes size. */
  setPlanes(planes: Planes): void;
  /** A byte range of a plane changed and must reach the tier's copy of it. */
  upload(plane: PlaneName, byteStart: number, byteEnd: number): void;
  present(info: PresentInfo): void;
  /** Call `listener` once if the tier stops working and cannot draw any more. */
  onLost(listener: (reason: string) => void): void;
  dispose(): void;
}
