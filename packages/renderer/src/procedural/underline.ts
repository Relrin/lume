/**
 * Underlines as shapes.
 *
 * Each underline style is drawn once, one cell wide, and kept in the atlas like a glyph.
 * A cell with an underline then needs a single instance whatever the style, where
 * drawing a dotted line from rectangles would need as many as it has dots.
 *
 * Every pattern repeats a whole number of times across a cell, so underlines on
 * neighbouring cells join into one line.
 */

import { CURLY_HEIGHT_RATIO, DASH_FRACTION } from "../constants.ts";
import { UnderlineStyle } from "../frame.ts";
import type { CellMetrics } from "../types.ts";
import type { Shape } from "./coverage.ts";

/** An underline, one cell wide. */
export interface UnderlineSprite {
  /** From the top of the cell to the top of the sprite. */
  top: number;
  height: number;
  shapes: Shape[];
}

/** The sprite for an underline style, or `undefined` for no underline. */
export function underlineSprite(style: number, metrics: CellMetrics): UnderlineSprite | undefined {
  const { cellWidth, cellHeight, lineThickness: t } = metrics;
  // Keep the sprite inside the cell, moving it up if the font puts underlines low.
  const place = (height: number, shapes: Shape[]): UnderlineSprite => ({
    top: Math.max(0, Math.min(metrics.underlineTop, cellHeight - height)),
    height,
    shapes,
  });
  const bar = (y: number, x0 = 0, x1 = cellWidth): Shape => ({
    kind: "rect",
    x0,
    y0: y,
    x1,
    y1: y + t,
  });

  switch (style) {
    case UnderlineStyle.Single:
      return place(t, [bar(0)]);
    case UnderlineStyle.Double:
      return place(3 * t, [bar(0), bar(2 * t)]);
    case UnderlineStyle.Dashed:
      return place(t, [bar(0, 0, Math.max(1, Math.round(cellWidth * DASH_FRACTION)))]);
    case UnderlineStyle.Dotted: {
      // Dots as wide as the line is thick, with the same again between them, adjusted
      // so that a whole number fits the cell.
      const count = Math.max(1, Math.round(cellWidth / (2 * t)));
      const period = cellWidth / count;
      const dots: Shape[] = [];
      for (let i = 0; i < count; i++) {
        const from = Math.round(i * period);
        dots.push(bar(0, from, Math.max(from + 1, Math.round(i * period + period / 2))));
      }
      return place(t, dots);
    }
    case UnderlineStyle.Curly: {
      // One full wave per cell. The stroke is drawn as short straight pieces, starting
      // and ending outside the cell so the ends are not cut square.
      const height = Math.max(t + 2, Math.round(cellHeight * CURLY_HEIGHT_RATIO));
      const amplitude = (height - t) / 2;
      const y = (x: number) => height / 2 + amplitude * Math.sin((2 * Math.PI * x) / cellWidth);
      const wave: Shape[] = [];
      for (let x = -1; x <= cellWidth; x++) {
        wave.push({ kind: "line", x0: x, y0: y(x), x1: x + 1, y1: y(x + 1), width: t });
      }
      return place(height, wave);
    }
    default:
      return undefined;
  }
}
