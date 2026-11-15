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
import { NO_SELECTION, type RenderFrame } from "./frame.ts";
import { buildOverlay, buildRow, type OverlayCounts } from "./instances.ts";
import { createPlanes, type PlaneName, type Planes } from "./layout.ts";
import { fontKey, gridSize, themeMetrics } from "./metrics.ts";
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
const CELL_PLANES: readonly PlaneName[] = ["background", "underline", "glyph", "line"];

/** Draws frames through a tier backend. */
export class GridRenderer implements CellRenderer {
  readonly #backend: TierBackend;
  readonly #fonts: FontBackend;
  #theme: TerminalTheme;
  #focused = true;
  #cursorVisible = true;
  #underlines: readonly UnderlineRange[] = [];
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
  #selectionStart = new Int32Array(0);
  #selectionEnd = new Int32Array(0);

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

  gridSize(): { cols: number; rows: number } {
    return gridSize(this.#surface, this.#ensureMetrics());
  }

  draw(frame: RenderFrame): void {
    if (this.#disposed) return;
    // A surface with no area has nothing to draw on, and some tiers reject it outright.
    if (this.#surface.width === 0 || this.#surface.height === 0) return;

    const metrics = this.#ensureMetrics();
    const atlas = this.#atlas!;
    if (this.#surfaceStale) {
      this.#backend.configure(this.#surface.width, this.#surface.height);
      this.#surfaceStale = false;
      this.#force = true;
    }
    const planes = this.#ensurePlanes(frame);
    if (atlas.generation !== this.#generation) this.#force = true;

    const { rows } = frame;
    let count = collectDirtyRows(
      frame,
      this.#force,
      this.#selectionStart,
      this.#selectionEnd,
      this.#dirty,
    );
    const generation = atlas.generation;
    for (let row = 0; row < rows; row++) {
      if (this.#dirty[row] === 1) buildRow(frame, row, planes, atlas, metrics, this.#theme);
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
        buildRow(frame, row, planes, atlas, metrics, this.#theme);
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

    this.#upload(planes, frame.cols, count, rebuiltAll);
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
    this.#selectionStart = new Int32Array(frame.rows).fill(NO_SELECTION);
    this.#selectionEnd = new Int32Array(frame.rows);
    this.#backend.setPlanes(planes);
    this.#force = true;
    return planes;
  }

  /** Send the rows that were rebuilt to the tier. */
  #upload(planes: Planes, cols: number, count: number, all: boolean): void {
    if (count === 0) return;
    const whole = all || count > this.#rows * FULL_UPLOAD_DIRTY_RATIO;
    const runs = whole ? [] : rowRuns(this.#dirty, this.#rows);
    for (const name of CELL_PLANES) {
      const plane = planes[name];
      if (whole) {
        this.#backend.upload(name, 0, plane.bytes.byteLength);
        continue;
      }
      const rowBytes = cols * plane.perCell * plane.layout.stride;
      for (let i = 0; i < runs.length; i += 2) {
        this.#backend.upload(name, runs[i]! * rowBytes, runs[i + 1]! * rowBytes);
      }
    }
  }
}
