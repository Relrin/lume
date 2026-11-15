/**
 * Every tuning value of the renderer, each defined once.
 *
 * The visual ratios are starting points chosen by eye against the bundled test font; the
 * performance values are to be confirmed by measurement.
 */

import type { PageKind, Tier } from "./types.ts";

/** The order tiers are tried in. The first one that works is used. */
export const TIER_ORDER: readonly Tier[] = ["webgpu", "webgl2", "canvas2d"];

/** Width and height of one atlas page, in pixels. */
export const ATLAS_PAGE_SIZE = 2048;

/**
 * The most pages of each kind (coverage masks, colour glyphs) an atlas grows to. When
 * they are full the atlas is emptied and rebuilt from what is on screen.
 */
export const ATLAS_MAX_PAGES = 4;

/** Bytes one pixel of an atlas page takes: coverage is one byte, colour is four. */
export const ATLAS_BYTES_PER_PIXEL: Readonly<Record<PageKind, number>> = { mask: 1, color: 4 };

/** Empty pixels kept between glyphs in a page, so neighbours never bleed into each other. */
export const ATLAS_GLYPH_GAP = 1;

/** A double-width character spans this many cells; nothing spans more. */
export const MAX_CELL_SPAN = 2;

/**
 * How far a glyph may reach outside its cells on each side, as a fraction of the cell
 * width. Italics and some fallback fonts lean past the cell; this is the room they get.
 */
export const GLYPH_OVERHANG_RATIO = 0.5;

/** Pixels more transparent than this do not count as ink when a glyph is measured. */
export const CLASSIFY_MIN_ALPHA = 16;

/**
 * A glyph is drawn in white. If every inked pixel stays within this distance of white
 * the glyph is a plain shape to be tinted; otherwise it brought its own colours.
 */
export const CLASSIFY_WHITE_TOLERANCE = 8;

/** Rectangle slots per cell for strikethrough and overline. */
export const LINE_RECTS_PER_CELL = 2;

/** Rectangles the cursor can need: a hollow block is four. */
export const CURSOR_RECTS_MAX = 4;

/**
 * Runs of cells that can be underlined at once on top of the grid, as the link under
 * the pointer is. A link wraps over a few rows at most; runs beyond this are not drawn.
 */
export const UNDERLINE_RANGES_MAX = 8;

/** Rectangles drawn on top of the grid: the cursor's, then one per underlined run. */
export const OVERLAY_RECT_CAPACITY = CURSOR_RECTS_MAX + UNDERLINE_RANGES_MAX;

/** Glyphs redrawn on top of a block cursor. */
export const OVERLAY_GLYPH_CAPACITY = 1;

/**
 * When more than this fraction of the rows changed, a whole buffer is uploaded in one
 * call instead of one call per run of rows.
 */
export const FULL_UPLOAD_DIRTY_RATIO = 0.5;

/** Line height as a multiple of the font's own ascent plus descent. */
export const DEFAULT_LINE_HEIGHT = 1.2;

/** Thickness of underlines, strikethrough and light box-drawing lines, per cell height. */
export const LINE_THICKNESS_RATIO = 0.06;

/** How far below the baseline an underline sits, as a fraction of the descent. */
export const UNDERLINE_POSITION_RATIO = 0.4;

/** How far above the baseline a strikethrough sits, as a fraction of the ascent. */
export const STRIKETHROUGH_POSITION_RATIO = 0.32;

/** Height of a curly underline's wave, per cell height. */
export const CURLY_HEIGHT_RATIO = 0.16;

/** The part of a cell's width a dashed underline's dash covers. */
export const DASH_FRACTION = 0.5;

/** Heavy box-drawing lines are this many times as thick as light ones. */
export const BOX_HEAVY_FACTOR = 2;

/** Coverage of the light, medium and dark shade blocks. */
export const SHADE_ALPHAS: readonly [number, number, number] = [0.25, 0.5, 0.75];

/**
 * Samples per pixel edge when a curve or diagonal of a drawn glyph is turned into
 * coverage. Four gives sixteen levels, which is as smooth as text antialiasing.
 */
export const PROCEDURAL_SUPERSAMPLE = 4;

/** Width of a bar cursor, per cell width. */
export const CURSOR_BAR_WIDTH_RATIO = 0.12;

/**
 * A terminal is idle most of the time, so the adapter that draws less power is asked
 * for. Either adapter draws a terminal in well under a frame.
 */
export const WEBGPU_POWER_PREFERENCE: "low-power" | "high-performance" = "low-power";

/**
 * Refuse a WebGL2 context that would be drawn by software. Canvas 2D is the better
 * last resort than a software rasteriser pretending to be a GPU.
 */
export const WEBGL_FAIL_IF_MAJOR_PERFORMANCE_CAVEAT = true;

/**
 * The most WebGL2 contexts a window keeps alive at once. A browser allows a page only
 * so many and, past its limit, takes the oldest away without asking. A pane that would
 * be one too many draws with Canvas 2D instead, and no pane loses its picture.
 */
export const WEBGL2_CONTEXTS_MAX = 12;
