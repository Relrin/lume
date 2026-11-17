/**
 * Cell geometry from font measurements.
 *
 * Everything is rounded to whole device pixels here, once, so that every cell is the
 * same size and every line lands on a pixel boundary. Positions computed from these
 * numbers need no further rounding.
 */

import {
  DEFAULT_LINE_HEIGHT,
  LINE_THICKNESS_RATIO,
  STRIKETHROUGH_POSITION_RATIO,
  UNDERLINE_POSITION_RATIO,
} from "./constants.ts";
import type {
  CellMetrics,
  FontBackend,
  FontSpec,
  RawFontMetrics,
  Size,
  TerminalTheme,
} from "./types.ts";

/** The cell geometry for a measured font. `padding` is in CSS pixels. */
export function cellMetrics(
  raw: RawFontMetrics,
  font: FontSpec,
  padding: number,
  dpr: number,
): CellMetrics {
  const cellWidth = Math.max(1, Math.round(raw.advance));
  const textHeight = raw.ascent + raw.descent;
  const cellHeight = Math.max(1, Math.round(textHeight * (font.lineHeight ?? DEFAULT_LINE_HEIGHT)));
  // The extra line height is split evenly above and below the text.
  const baseline = Math.round((cellHeight - textHeight) / 2 + raw.ascent);
  const lineThickness = Math.max(1, Math.round(cellHeight * LINE_THICKNESS_RATIO));

  const underline = baseline + Math.round(raw.descent * UNDERLINE_POSITION_RATIO);
  // A double underline needs three thicknesses below this; keep it inside the cell.
  const underlineTop = Math.max(0, Math.min(underline, cellHeight - lineThickness));
  // A line shorter than its letters puts the baseline below the cell; both lines stay in it.
  const strikethroughTop = Math.max(
    0,
    Math.min(
      cellHeight - lineThickness,
      baseline - Math.round(raw.ascent * STRIKETHROUGH_POSITION_RATIO) - (lineThickness >> 1),
    ),
  );

  return {
    cellWidth,
    cellHeight,
    baseline,
    lineThickness,
    underlineTop,
    strikethroughTop,
    padding: Math.max(0, Math.round(padding * dpr)),
  };
}

/**
 * The cell geometry of a theme at a device pixel ratio, with the font measured by
 * `fonts`. A renderer and whoever asks what a renderer would measure both go through
 * here, so the two cannot disagree.
 */
export function themeMetrics(fonts: FontBackend, theme: TerminalTheme, dpr: number): CellMetrics {
  return cellMetrics(fonts.measure(theme.font, dpr), theme.font, theme.padding, dpr);
}

/** The whole cells that fit a surface, at least one each way. `surface` is in device pixels. */
export function gridSize(surface: Size, metrics: CellMetrics): { cols: number; rows: number } {
  const usable = (extent: number, cell: number) =>
    Math.max(1, Math.floor((extent - 2 * metrics.padding) / cell));
  return {
    cols: usable(surface.width, metrics.cellWidth),
    rows: usable(surface.height, metrics.cellHeight),
  };
}

/** A string that changes whenever the pixels of a glyph would. */
export function fontKey(font: FontSpec, dpr: number, metrics: CellMetrics): string {
  return [
    font.family,
    font.sizePx,
    font.weight ?? "",
    font.boldWeight ?? "",
    dpr,
    metrics.cellWidth,
    metrics.cellHeight,
    metrics.baseline,
    metrics.lineThickness,
  ].join("|");
}
