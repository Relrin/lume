/**
 * The renderer every tier shares.
 *
 * `GridRenderer` does everything about drawing a terminal that does not depend on what
 * it is drawn with: cell geometry, the atlas, which rows to rebuild, building the
 * instance records, and which bytes to send on. A tier supplies a `TierBackend` that
 * stores the bytes and draws them.
 *
 * It never keeps a frame. A frame's arrays belong to whoever produced it and can be
 * replaced between calls, so everything needed later is copied out during `draw`.
 */

import { GlyphAtlas } from "./atlas/atlas.ts";
import { FULL_UPLOAD_DIRTY_RATIO } from "./constants.ts";
import { collectDirtyRows, rowRuns } from "./dirty.ts";
import { HIGHLIGHT_STRIDE, NO_SELECTION, type Highlights, type RenderFrame } from "./frame.ts";
import { buildOverlay, buildRow, RowUse, type OverlayCounts } from "./instances.ts";
import { createPlanes, type Planes } from "./layout.ts";
import { fontKey, gridOrigin, gridSize, themeMetrics } from "./metrics.ts";
import type {
  CellMetrics,
  CellRenderer,
  FontBackend,
  Size,
  TerminalTheme,
  Tier,
  TierBackend,
  UnderlineRange,
} from "./types.ts";

/** The planes with a fixed number of records per cell. */
const CELL_PLANES = ["background", "underline", "glyph", "line"] as const;
type CellPlane = (typeof CELL_PLANES)[number];

/** The `RowUse` bit of each plane laid out by cell. */
const USE_BIT: Readonly<Record<CellPlane, number>> = {
  background: RowUse.Background,
  underline: RowUse.Underline,
  glyph: RowUse.Glyph,
  line: RowUse.Line,
};

/** Draws frames through a tier backend. */
export class GridRenderer implements CellRenderer {
  readonly #backend: TierBackend;
  readonly #fonts: FontBackend;
  #theme: TerminalTheme;
  #focused = true;
  #cursorVisible = true;
  #underlines: readonly UnderlineRange[] = [];
  #centred = false;
  /** Where the grid was last built from, to notice when that moves. */
  #originKey = "";
  #disposed = false;

  /** The surface in CSS pixels and the device pixel ratio, as last set. */
  #size: Size = { width: 0, height: 0 };
  #dpr = 1;
  /** The surface in device pixels. */
  #surface: Size = { width: 0, height: 0 };
  #surfaceStale = true;

  #metrics: CellMetrics | undefined;
  #atlas: GlyphAtlas | undefined;
  /** The atlas generation the current records were built against. */
  #generation = -1;

  #planes: Planes | undefined;
  #cols = 0;
  #rows = 0;
  #dirty = new Uint8Array(0);
  #repaint = new Uint8Array(0);
  /** For each row, the `RowUse` bits of the planes it has records in. */
  #rowUse = new Uint8Array(0);
  /**
   * For each plane laid out by cell, how many rows from the top the backend's copy may
   * hold records in: below them it holds only empty ones.
   */
  #uploadedRows: Record<CellPlane, number> = { background: 0, underline: 0, glyph: 0, line: 0 };
  #selectionStart = new Int32Array(0);
  #selectionEnd = new Int32Array(0);

  /** The highlighted runs as last given, and whether `#marks` has caught up with them. */
  #highlights = new Int32Array(0);
  #highlightCount = 0;
  #highlightsStale = false;
  /** Per cell: the `HighlightKind` its row was last built with, or zero. */
  #marks = new Uint8Array(0);
  /** Scratch of the same size, for working out the next `#marks`. */
  #nextMarks = new Uint8Array(0);
  /** Per row: 1 when its marks changed since it was built. */
  #markRows = new Uint8Array(0);

  /** Everything must be rebuilt and redrawn on the next frame. */
  #force = true;
  /** What was last drawn on top of the grid, to notice when it changes, and its rows. */
  #overlayKey = "";
  #overlayRows: number[] = [];
  #overlay: OverlayCounts = { rects: 0, glyphs: 0 };

  constructor(backend: TierBackend, fonts: FontBackend, theme: TerminalTheme) {
    this.#backend = backend;
    this.#fonts = fonts;
    this.#theme = theme;
  }

  get tier(): Tier {
    return this.#backend.tier;
  }

  get metrics(): CellMetrics {
    return this.#ensureMetrics();
  }

  resize(px: Size, dpr: number): void {
    if (dpr !== this.#dpr) {
      this.#dpr = dpr;
      // Glyphs are drawn for one pixel ratio; a different one needs them all again.
      this.#metrics = undefined;
    }
    this.#size = { width: px.width, height: px.height };
    const surface = {
      width: Math.max(0, Math.round(px.width * dpr)),
      height: Math.max(0, Math.round(px.height * dpr)),
    };
    if (surface.width !== this.#surface.width || surface.height !== this.#surface.height) {
      this.#surface = surface;
      // Changing the size of a surface clears it, so it is applied when the next frame
      // is drawn and not before, or the terminal would flash empty in between.
      this.#surfaceStale = true;
    }
  }

  setTheme(theme: TerminalTheme): void {
    const before = this.#theme.font;
    const after = theme.font;
    const fontChanged =
      before.family !== after.family ||
      before.sizePx !== after.sizePx ||
      before.lineHeight !== after.lineHeight ||
      before.weight !== after.weight ||
      before.boldWeight !== after.boldWeight ||
      this.#theme.padding !== theme.padding;
    this.#theme = theme;
    if (fontChanged) this.#metrics = undefined;
    this.#force = true;
  }

  setFocused(focused: boolean): void {
    this.#focused = focused;
  }

  setCursorVisible(visible: boolean): void {
    this.#cursorVisible = visible;
  }

  setUnderlines(ranges: readonly UnderlineRange[]): void {
    // Copied: the caller's list is its own to change.
    this.#underlines = ranges.map((range) => ({ ...range }));
  }

  setHighlights(highlights: Highlights | undefined): void {
    const count = highlights?.count ?? 0;
    if (count === 0 && this.#highlightCount === 0) return;
    const length = count * HIGHLIGHT_STRIDE;
    // Copied: the caller's list is its own to refill.
    if (this.#highlights.length < length) this.#highlights = new Int32Array(length);
    if (highlights) this.#highlights.set(highlights.data.subarray(0, length));
    this.#highlightCount = count;
    this.#highlightsStale = true;
  }

  gridSize(): { cols: number; rows: number } {
    return gridSize(this.#surface, this.#ensureMetrics());
  }

  setCentred(centred: boolean): void {
    this.#centred = centred;
  }

  origin(cols: number, rows: number): { x: number; y: number } {
    return gridOrigin(this.#surface, this.#ensureMetrics(), cols, rows, this.#centred);
  }

  draw(frame: RenderFrame): void {
    if (this.#disposed) return;
    // A surface with no area has nothing to draw on, and some tiers reject it outright.
    if (this.#surface.width === 0 || this.#surface.height === 0) return;

    const measured = this.#ensureMetrics();
    const origin = gridOrigin(this.#surface, measured, frame.cols, frame.rows, this.#centred);
    const metrics =
      origin.x === measured.originX && origin.y === measured.originY
        ? measured
        : { ...measured, originX: origin.x, originY: origin.y };
    const originKey = `${origin.x},${origin.y}`;
    // Every record holds where it is drawn, so a grid that moves is built again.
    if (originKey !== this.#originKey) {
      this.#originKey = originKey;
      this.#force = true;
    }
    const atlas = this.#atlas!;
    if (this.#surfaceStale) {
      this.#backend.configure(this.#surface.width, this.#surface.height);
      this.#surfaceStale = false;
      this.#force = true;
    }
    const planes = this.#ensurePlanes(frame);
    if (atlas.generation !== this.#generation) this.#force = true;
    if (this.#highlightsStale) this.#markHighlights(frame.cols, frame.rows);
    const marks = this.#marks;

    const { rows } = frame;
    let count = collectDirtyRows(
      frame,
      this.#force,
      this.#selectionStart,
      this.#selectionEnd,
      this.#dirty,
      this.#markRows,
    );
    const generation = atlas.generation;
    const rowUse = this.#rowUse;
    for (let row = 0; row < rows; row++) {
      if (this.#dirty[row] === 1) {
        rowUse[row] = buildRow(frame, row, planes, atlas, metrics, this.#theme, marks);
      }
    }
    const buildOverlays = () =>
      buildOverlay(
        frame,
        planes,
        atlas,
        metrics,
        this.#theme,
        this.#focused,
        this.#cursorVisible,
        this.#underlines,
      );
    let overlay = buildOverlays();
    let rebuiltAll = this.#force;
    if (atlas.generation !== generation) {
      // The atlas filled up and was emptied while rows were being built, so records
      // built earlier point at glyphs that are gone. Building everything once more
      // refills it with exactly what is on screen. If even that does not fit, some
      // glyphs stay missing until the screen changes; it is not tried again.
      this.#dirty.fill(1);
      count = rows;
      rebuiltAll = true;
      for (let row = 0; row < rows; row++) {
        rowUse[row] = buildRow(frame, row, planes, atlas, metrics, this.#theme, marks);
      }
      overlay = buildOverlays();
    }
    this.#generation = atlas.generation;

    const cursor = frame.cursor;
    const cursorShown = cursor.visible && this.#cursorVisible;
    const overlayRows: number[] = [];
    let overlayKey = "";
    if (cursorShown) {
      overlayKey = `${cursor.x},${cursor.y},${cursor.shape},${cursor.wideTail},${cursor.color},${this.#focused}`;
      overlayRows.push(cursor.y);
    }
    for (const range of this.#underlines) {
      overlayKey += `|${range.row},${range.startCol},${range.endCol}`;
      overlayRows.push(range.row);
    }
    const overlayChanged = overlayKey !== this.#overlayKey;
    if (count === 0 && !overlayChanged && !rebuiltAll) return;

    const used = this.#usedRows(rows);
    this.#upload(planes, frame.cols, count, rebuiltAll, used);
    if (overlayChanged || rebuiltAll || count > 0) {
      this.#backend.upload("overlayRect", 0, planes.overlayRect.bytes.byteLength);
      this.#backend.upload("overlayGlyph", 0, planes.overlayGlyph.bytes.byteLength);
    }

    // A surface that keeps its pixels must also redraw the rows the overlay left and the
    // rows it is on now, whether or not their cells changed.
    let repaint: Uint8Array | undefined;
    if (!rebuiltAll) {
      repaint = this.#repaint;
      repaint.set(this.#dirty);
      for (const row of this.#overlayRows) if (row >= 0 && row < rows) repaint[row] = 1;
      for (const row of overlayRows) if (row >= 0 && row < rows) repaint[row] = 1;
    }
    this.#overlayKey = overlayKey;
    this.#overlayRows = overlayRows;
    this.#overlay = overlay;
    this.#force = false;

    this.#backend.present({
      width: this.#surface.width,
      height: this.#surface.height,
      clear: frame.background,
      planes,
      cols: frame.cols,
      rows,
      metrics,
      repaint,
      overlayRects: overlay.rects,
      overlayGlyphs: overlay.glyphs,
      used: {
        background: used.background * frame.cols * planes.background.perCell,
        underline: used.underline * frame.cols * planes.underline.perCell,
        glyph: used.glyph * frame.cols * planes.glyph.perCell,
        line: used.line * frame.cols * planes.line.perCell,
      },
    });
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#backend.dispose();
  }

  /** How many overlay records the last frame used. For tests and diagnostics. */
  get overlay(): OverlayCounts {
    return this.#overlay;
  }

  /** The CSS size and device pixel ratio last given, to carry over to another tier. */
  get placement(): { size: Size; dpr: number } {
    return { size: this.#size, dpr: this.#dpr };
  }

  #ensureMetrics(): CellMetrics {
    if (this.#metrics) return this.#metrics;
    const font = this.#theme.font;
    const metrics = themeMetrics(this.#fonts, this.#theme, this.#dpr);
    this.#metrics = metrics;
    this.#atlas = this.#backend.atlas(
      fontKey(font, this.#dpr, metrics),
      (sink) => new GlyphAtlas(sink, this.#fonts.rasteriser(font, this.#dpr, metrics), metrics),
    );
    this.#generation = -1;
    this.#force = true;
    return metrics;
  }

  #ensurePlanes(frame: RenderFrame): Planes {
    if (this.#planes && frame.cols === this.#cols && frame.rows === this.#rows) {
      return this.#planes;
    }
    const planes = createPlanes(frame.cols * frame.rows);
    this.#planes = planes;
    this.#cols = frame.cols;
    this.#rows = frame.rows;
    this.#dirty = new Uint8Array(frame.rows);
    this.#repaint = new Uint8Array(frame.rows);
    this.#rowUse = new Uint8Array(frame.rows);
    // New planes start out empty in the backend too.
    this.#uploadedRows = { background: 0, underline: 0, glyph: 0, line: 0 };
    this.#selectionStart = new Int32Array(frame.rows).fill(NO_SELECTION);
    this.#selectionEnd = new Int32Array(frame.rows);
    this.#marks = new Uint8Array(frame.cols * frame.rows);
    this.#nextMarks = new Uint8Array(frame.cols * frame.rows);
    this.#markRows = new Uint8Array(frame.rows);
    // The runs were laid out for a grid of another size.
    this.#highlightsStale = this.#highlightCount > 0;
    this.#backend.setPlanes(planes);
    this.#force = true;
    return planes;
  }

  /**
   * Bring `#marks` up to date with the highlighted runs, and flag the rows where a cell
   * gained, lost or changed its highlight: a highlight changes a cell's colours without
   * the cell itself changing.
   */
  #markHighlights(cols: number, rows: number): void {
    this.#highlightsStale = false;
    const next = this.#nextMarks;
    next.fill(0);
    const runs = this.#highlights;
    for (let at = 0; at < this.#highlightCount * HIGHLIGHT_STRIDE; at += HIGHLIGHT_STRIDE) {
      const row = runs[at]!;
      if (row < 0 || row >= rows) continue;
      const first = Math.max(0, runs[at + 1]!);
      const last = Math.min(cols - 1, runs[at + 2]!);
      const kind = runs[at + 3]!;
      // Where two runs meet on a cell the greater kind is kept: the match a search is
      // at shows over the others.
      for (let cell = row * cols + first; cell <= row * cols + last; cell++) {
        if (kind > next[cell]!) next[cell] = kind;
      }
    }
    const before = this.#marks;
    for (let row = 0; row < rows; row++) {
      for (let cell = row * cols; cell < (row + 1) * cols; cell++) {
        if (next[cell] !== before[cell]) {
          this.#markRows[row] = 1;
          break;
        }
      }
    }
    this.#marks = next;
    this.#nextMarks = before;
  }

  /** Send the rows that were rebuilt to the tier. */
  /** For each plane laid out by cell, the rows down to the last that has records in it. */
  #usedRows(rows: number): Record<CellPlane, number> {
    const used = { background: 0, underline: 0, glyph: 0, line: 0 };
    const rowUse = this.#rowUse;
    let missing = RowUse.Background | RowUse.Underline | RowUse.Glyph | RowUse.Line;
    for (let row = rows - 1; row >= 0 && missing !== 0; row--) {
      const bits = rowUse[row]! & missing;
      if (bits === 0) continue;
      for (const name of CELL_PLANES) {
        if ((bits & USE_BIT[name]) !== 0) used[name] = row + 1;
      }
      missing &= ~bits;
    }
    return used;
  }

  /**
   * Hands the backend the records that changed. When most rows changed a plane goes in
   * one piece, but only as far down as it has records, or had them in the backend's copy:
   * the rest of it is empty on both sides.
   */
  #upload(
    planes: Planes,
    cols: number,
    count: number,
    all: boolean,
    used: Readonly<Record<CellPlane, number>>,
  ): void {
    if (count === 0) return;
    const whole = all || count > this.#rows * FULL_UPLOAD_DIRTY_RATIO;
    const runs = whole ? [] : rowRuns(this.#dirty, this.#rows);
    for (const name of CELL_PLANES) {
      const plane = planes[name];
      const rowBytes = cols * plane.perCell * plane.layout.stride;
      if (whole) {
        const rows = Math.max(used[name], this.#uploadedRows[name]);
        if (rows > 0) this.#backend.upload(name, 0, rows * rowBytes);
        this.#uploadedRows[name] = used[name];
        continue;
      }
      for (let i = 0; i < runs.length; i += 2) {
        this.#backend.upload(name, runs[i]! * rowBytes, runs[i + 1]! * rowBytes);
      }
      this.#uploadedRows[name] = Math.max(this.#uploadedRows[name], used[name]);
    }
  }
}
