/**
 * Turning a frame into instance records.
 *
 * This is the one place that decides what a cell looks like: where its background,
 * glyph, underline and strike lines go, in which colours, and how the selection and the
 * cursor change them. Every tier draws the records produced here, so they cannot
 * disagree about any of it.
 */

import type { GlyphAtlas } from "./atlas/atlas.ts";
import { CURSOR_BAR_WIDTH_RATIO, LINE_RECTS_PER_CELL } from "./constants.ts";
import {
  CellFlag,
  CursorShape,
  GLYPH_CLUSTER_TAG,
  NO_SELECTION,
  UNDERLINE_MASK,
  UNDERLINE_SHIFT,
  type RenderFrame,
} from "./frame.ts";
import { clearSlots, GLYPH_FLAG_COLOR, writeGlyph, writeRect, type Planes } from "./layout.ts";
import type { CellMetrics, TerminalTheme } from "./types.ts";

/** The atlas slot of the glyph in a cell, or the empty slot. */
function glyphSlot(frame: RenderFrame, index: number, atlas: GlyphAtlas): number {
  const glyph = frame.glyph[index]!;
  if (glyph === 0) return 0;
  const flags = frame.flags[index]!;
  const bold = (flags & CellFlag.Bold) !== 0;
  const italic = (flags & CellFlag.Italic) !== 0;
  const span = (flags & CellFlag.Wide) !== 0 ? 2 : 1;
  if ((glyph & GLYPH_CLUSTER_TAG) !== 0) {
    const text = frame.clusters[glyph & ~GLYPH_CLUSTER_TAG];
    return text === undefined ? 0 : atlas.cluster(text, bold, italic, span);
  }
  return atlas.glyph(glyph, bold, italic, span);
}

/** Rebuild the records of one row of the grid. */
export function buildRow(
  frame: RenderFrame,
  row: number,
  planes: Planes,
  atlas: GlyphAtlas,
  metrics: CellMetrics,
  theme: TerminalTheme,
): void {
  const { cols, glyph, fg, bg, underlineColor, flags } = frame;
  const { cellWidth, cellHeight, lineThickness, padding } = metrics;
  const first = row * cols;
  const y = padding + row * cellHeight;

  clearSlots(planes.background, first, cols);
  clearSlots(planes.underline, first, cols);
  clearSlots(planes.glyph, first, cols);
  clearSlots(planes.line, first * LINE_RECTS_PER_CELL, cols * LINE_RECTS_PER_CELL);

  const selectionStart = frame.selectionStart[row]!;
  const selectionEnd = selectionStart === NO_SELECTION ? -1 : frame.selectionEnd[row]!;

  for (let col = 0; col < cols; col++) {
    const index = first + col;
    const x = padding + col * cellWidth;
    const cellFlags = flags[index]!;
    const selected =
      col >= selectionStart && col <= selectionEnd && selectionStart !== NO_SELECTION;

    // The selection replaces the cell's colours, so selected text can have a colour of
    // its own instead of whatever it had before.
    const background = selected ? theme.selectionBackground : bg[index]!;
    let foreground = fg[index]!;
    let underline = underlineColor[index]!;
    if (selected && theme.selectionForeground !== undefined) {
      foreground = theme.selectionForeground;
      underline = theme.selectionForeground;
    }

    // A fully transparent background is the default one, which is already there.
    if (background >>> 24 !== 0) {
      writeRect(planes.background, index, x, y, cellWidth, cellHeight, background);
    }

    const style = (cellFlags >> UNDERLINE_SHIFT) & UNDERLINE_MASK;
    if (style !== 0) {
      const slot = atlas.underline(style);
      if (slot !== 0) {
        writeGlyph(
          planes.underline,
          index,
          x,
          y + atlas.offsetY[slot]!,
          atlas.width[slot]!,
          atlas.height[slot]!,
          atlas.u[slot]!,
          atlas.v[slot]!,
          atlas.page[slot]!,
          0,
          underline,
        );
      }
    }

    if (glyph[index] !== 0 && (cellFlags & CellFlag.Spacer) === 0) {
      const slot = glyphSlot(frame, index, atlas);
      if (slot !== 0) {
        writeGlyph(
          planes.glyph,
          index,
          x + atlas.offsetX[slot]!,
          y + atlas.offsetY[slot]!,
          atlas.width[slot]!,
          atlas.height[slot]!,
          atlas.u[slot]!,
          atlas.v[slot]!,
          atlas.page[slot]!,
          atlas.color[slot] === 1 ? GLYPH_FLAG_COLOR : 0,
          foreground,
        );
      }
    }

    if ((cellFlags & (CellFlag.Strikethrough | CellFlag.Overline)) !== 0) {
      const lines = index * LINE_RECTS_PER_CELL;
      if ((cellFlags & CellFlag.Strikethrough) !== 0) {
        const top = y + metrics.strikethroughTop;
        writeRect(planes.line, lines, x, top, cellWidth, lineThickness, foreground);
      }
      if ((cellFlags & CellFlag.Overline) !== 0) {
        writeRect(planes.line, lines + 1, x, y, cellWidth, lineThickness, foreground);
      }
    }
  }
}

/** How many overlay records the cursor uses. */
export interface OverlayCounts {
  rects: number;
  glyphs: number;
}

/**
 * Rebuild the overlay planes: the cursor.
 *
 * A block cursor covers its cell, so the glyph under it is drawn again on top in a
 * colour that shows against the cursor. Without keyboard focus every shape becomes a
 * hollow block, which is how a terminal shows that typing goes elsewhere.
 */
export function buildCursor(
  frame: RenderFrame,
  planes: Planes,
  atlas: GlyphAtlas,
  metrics: CellMetrics,
  theme: TerminalTheme,
  focused: boolean,
): OverlayCounts {
  const { overlayRect, overlayGlyph } = planes;
  clearSlots(overlayRect, 0, overlayRect.capacity);
  clearSlots(overlayGlyph, 0, overlayGlyph.capacity);

  const cursor = frame.cursor;
  if (!cursor.visible || cursor.x >= frame.cols || cursor.y >= frame.rows) {
    return { rects: 0, glyphs: 0 };
  }

  const { cellWidth, cellHeight, lineThickness: t, padding } = metrics;
  // On the second half of a double-width character the cursor covers the whole character.
  const col = cursor.wideTail && cursor.x > 0 ? cursor.x - 1 : cursor.x;
  const index = cursor.y * frame.cols + col;
  const wide = cursor.wideTail || (frame.flags[index]! & CellFlag.Wide) !== 0;
  const x = padding + col * cellWidth;
  const y = padding + cursor.y * cellHeight;
  const width = wide ? cellWidth * 2 : cellWidth;
  const color = cursor.hasColor ? cursor.color : theme.cursor;
  const shape = focused ? cursor.shape : CursorShape.HollowBlock;

  switch (shape) {
    case CursorShape.Bar: {
      const bar = Math.max(1, Math.round(cellWidth * CURSOR_BAR_WIDTH_RATIO));
      writeRect(overlayRect, 0, x, y, bar, cellHeight, color);
      return { rects: 1, glyphs: 0 };
    }
    case CursorShape.Underline:
      writeRect(overlayRect, 0, x, y + cellHeight - t, width, t, color);
      return { rects: 1, glyphs: 0 };
    case CursorShape.HollowBlock:
      writeRect(overlayRect, 0, x, y, width, t, color);
      writeRect(overlayRect, 1, x, y + cellHeight - t, width, t, color);
      writeRect(overlayRect, 2, x, y + t, t, cellHeight - 2 * t, color);
      writeRect(overlayRect, 3, x + width - t, y + t, t, cellHeight - 2 * t, color);
      return { rects: 4, glyphs: 0 };
    default: {
      writeRect(overlayRect, 0, x, y, width, cellHeight, color);
      const slot = glyphSlot(frame, index, atlas);
      if (slot === 0) return { rects: 1, glyphs: 0 };
      writeGlyph(
        overlayGlyph,
        0,
        x + atlas.offsetX[slot]!,
        y + atlas.offsetY[slot]!,
        atlas.width[slot]!,
        atlas.height[slot]!,
        atlas.u[slot]!,
        atlas.v[slot]!,
        atlas.page[slot]!,
        atlas.color[slot] === 1 ? GLYPH_FLAG_COLOR : 0,
        theme.cursorText ?? frame.background,
      );
      return { rects: 1, glyphs: 1 };
    }
  }
}
