/**
 * Reading the screen for drawing.
 *
 * The library keeps a render state: a copy of what one frame needs, updated from the
 * terminal on request, with a flag per row saying whether the row changed. This module
 * turns that into a `RenderFrame`, re-reading only the rows that changed.
 *
 * Calls into the module are the expensive part, so a row is read in bulk: one call
 * returns the packed value of every cell, which is decoded here. A packed cell names its
 * style by a number instead of carrying it, and the library offers no lookup from that
 * number, so the style is fetched through the first cell of the row that uses it and
 * remembered for the rest of the row.
 *
 * Colours are resolved here, once, so that every way of drawing a frame gets the same
 * colours: the palette and the default colours are applied, a cell's own background is
 * taken into account, reverse video swaps the pair, and faint text is mixed towards the
 * background.
 */

import { NO_VALUE, OUT_OF_SPACE } from "./error.ts";
import {
  CellFlag,
  CursorShape,
  DirtyState,
  GLYPH_CLUSTER_TAG,
  NO_SELECTION,
  rgba,
  UNDERLINE_SHIFT,
  UnderlineStyle,
  type RenderFrame,
} from "./frame.ts";
import { bitReader } from "./manifest.ts";
import type { Core } from "./module.ts";
import type { Terminal } from "./terminal.ts";

/** How much of the foreground faint text keeps; the rest is the background. */
const FAINT_OPACITY = 0.5;

/**
 * The most multi-codepoint cell texts a frame remembers. Past it the table is emptied
 * and every row is read again, which keeps a long session of changing emoji bounded.
 */
const CLUSTER_POOL_LIMIT = 4096;

/** Codepoints read for one cell. A longer cluster is cut to this many. */
const GRAPHEME_CAP = 32;

/** Entries the per-row style table starts with. */
const STYLE_SLOTS = 16;

// Bits kept beside a style's frame flags while a row is decoded. They sit above every
// `CellFlag` bit and never reach the frame.
const STYLE_FG_SET = 1 << 10;
const STYLE_BG_SET = 1 << 11;
const STYLE_UNDERLINE_SET = 1 << 12;
const STYLE_INVERSE = 1 << 13;
const STYLE_FAINT = 1 << 14;
const STYLE_INVISIBLE = 1 << 15;
const FRAME_FLAGS = STYLE_FG_SET - 1;
const DECORATIONS =
  CellFlag.Strikethrough | CellFlag.Overline | (0x7 << UNDERLINE_SHIFT) | CellFlag.Hyperlink;

/** Reads a terminal's screen into a reusable frame. */
export class RenderState {
  /** The frame. The object is stable; its arrays are replaced when the grid is resized. */
  readonly frame: RenderFrame;

  readonly #core: Core;
  readonly #terminal: Terminal;
  #state: number;
  #iterator: number;
  #cells: number;

  // Scratch memory inside the module.
  readonly #scratch: number;
  readonly #scratchLen: number;
  readonly #out: number;
  readonly #iteratorSlot: number;
  readonly #cellsSlot: number;
  readonly #cellsView: number;
  readonly #selection: number;
  readonly #cursor: number;
  readonly #colors: number;
  readonly #style: number;
  readonly #codepoints: number;
  readonly #codepointBuffer: number;

  // The per-row style table: a style number to the slot holding its resolved form.
  readonly #styleSlots = new Map<number, number>();
  #styleFg = new Uint32Array(STYLE_SLOTS);
  #styleBg = new Uint32Array(STYLE_SLOTS);
  #styleUnderline = new Uint32Array(STYLE_SLOTS);
  #styleBits = new Uint16Array(STYLE_SLOTS);

  readonly #palette = new Uint32Array(256);
  /** The colours struct of the last update, to notice when a colour changed. */
  readonly #lastColors: Uint8Array;
  readonly #clusters: string[] = [];
  readonly #clusterIndex = new Map<string, number>();
  #first = true;

  // Keys, values, offsets and bit readers from the module's description of its types.
  readonly #k: Keys;

  constructor(terminal: Terminal) {
    const core = terminal.core;
    this.#core = core;
    this.#terminal = terminal;
    this.#k = keys(core);
    const k = this.#k;

    const align8 = (n: number) => (n + 7) & ~7;
    const sizes = [
      8,
      4,
      4,
      k.cellsViewSize,
      k.selectionSize,
      k.cursorSize,
      k.colorsSize,
      k.styleSize,
      GRAPHEME_CAP * 4,
      k.codepointBufferSize,
    ];
    this.#scratchLen = sizes.reduce((sum, size) => sum + align8(size), 0);
    this.#scratch = core.allocZeroed(this.#scratchLen);
    let offset = 0;
    const slots = sizes.map((size) => {
      const at = this.#scratch + offset;
      offset += align8(size);
      return at;
    }) as [number, number, number, number, number, number, number, number, number, number];
    [
      this.#out,
      this.#iteratorSlot,
      this.#cellsSlot,
      this.#cellsView,
      this.#selection,
      this.#cursor,
      this.#colors,
      this.#style,
      this.#codepoints,
      this.#codepointBuffer,
    ] = slots;
    this.#lastColors = new Uint8Array(k.colorsSize);

    this.#state = 0;
    this.#iterator = 0;
    this.#cells = 0;
    try {
      const { x } = core;
      this.#state = core.newHandle("ghostty_render_state_new", (out) =>
        x.ghostty_render_state_new(0, out),
      );
      this.#iterator = core.newHandle("ghostty_render_state_row_iterator_new", (out) =>
        x.ghostty_render_state_row_iterator_new(0, out),
      );
      this.#cells = core.newHandle("ghostty_render_state_row_cells_new", (out) =>
        x.ghostty_render_state_row_cells_new(0, out),
      );
    } catch (err) {
      this.free();
      throw err;
    }
    // The library fills an iterator it is handed the address of, so each handle is
    // kept in memory as well.
    const view = core.view();
    view.setUint32(this.#iteratorSlot, this.#iterator, true);
    view.setUint32(this.#cellsSlot, this.#cells, true);

    this.frame = {
      cols: 0,
      rows: 0,
      dirty: DirtyState.Full,
      dirtyRows: new Uint8Array(0),
      glyph: new Uint32Array(0),
      fg: new Uint32Array(0),
      bg: new Uint32Array(0),
      underlineColor: new Uint32Array(0),
      flags: new Uint16Array(0),
      selectionStart: new Int32Array(0),
      selectionEnd: new Int32Array(0),
      clusters: this.#clusters,
      cursor: {
        visible: false,
        x: 0,
        y: 0,
        shape: CursorShape.Block,
        blinking: false,
        wideTail: false,
        hasColor: false,
        color: 0,
      },
      foreground: 0,
      background: 0,
    };
  }

  /**
   * Bring the frame up to date with the terminal and return it.
   *
   * What changed is added to the frame's dirty state, which keeps accumulating until
   * `markDrawn`, so an update whose frame is never drawn loses nothing.
   */
  update(): RenderFrame {
    if (this.#state === 0) throw new Error("this render state has been freed");
    const core = this.#core;
    const { x } = core;
    const k = this.#k;
    const frame = this.frame;

    core.check(
      x.ghostty_render_state_update(this.#state, this.#terminal.handle),
      "ghostty_render_state_update",
    );

    const cols = this.#getU16(k.dataCols);
    const rows = this.#getU16(k.dataRows);
    this.#get(k.dataDirty, this.#out);
    const dirty = core.view().getInt32(this.#out, true);

    let full = this.#first || dirty === k.dirtyFull;
    this.#first = false;
    if (cols !== frame.cols || rows !== frame.rows) {
      this.#resize(cols, rows);
      full = true;
    }
    if (this.#readColors()) full = true;
    if (this.#clusters.length > CLUSTER_POOL_LIMIT) full = true;
    if (full) {
      // Every row is about to be read again, so no earlier cluster index survives.
      this.#clusters.length = 0;
      this.#clusterIndex.clear();
    }

    let changed = full;
    if (full || dirty !== k.dirtyFalse) {
      this.#get(k.dataRowIterator, this.#iteratorSlot);
      if (full) {
        for (let y = 0; y < rows && x.ghostty_render_state_row_iterator_next(this.#iterator); y++) {
          this.#readRow(y);
        }
      } else {
        while (x.ghostty_render_state_row_iterator_next_dirty(this.#iterator, this.#out)) {
          const y = core.view().getUint16(this.#out, true);
          if (y >= rows) continue;
          this.#readRow(y);
          frame.dirtyRows[y] = 1;
          changed = true;
        }
      }
    }
    // The dirty rows have been copied out; the frame carries them from here on.
    core.check(x.ghostty_render_state_clean(this.#state), "ghostty_render_state_clean");

    if (this.#readCursor()) changed = true;
    if (full) {
      frame.dirty = DirtyState.Full;
      frame.dirtyRows.fill(1);
    } else if (changed && frame.dirty === DirtyState.None) {
      frame.dirty = DirtyState.Partial;
    }
    return frame;
  }

  /** Say that the frame has been drawn, so its dirty state starts again from nothing. */
  markDrawn(): void {
    this.frame.dirty = DirtyState.None;
    this.frame.dirtyRows.fill(0);
  }

  /** Release the render state. The object must not be used afterwards. */
  free(): void {
    const { x } = this.#core;
    if (this.#cells !== 0) x.ghostty_render_state_row_cells_free(this.#cells);
    if (this.#iterator !== 0) x.ghostty_render_state_row_iterator_free(this.#iterator);
    if (this.#state !== 0) x.ghostty_render_state_free(this.#state);
    if (this.#state !== 0 || this.#iterator !== 0 || this.#cells !== 0) {
      this.#core.free(this.#scratch, this.#scratchLen);
    }
    this.#state = 0;
    this.#iterator = 0;
    this.#cells = 0;
  }

  #resize(cols: number, rows: number): void {
    const frame = this.frame;
    const cells = cols * rows;
    frame.cols = cols;
    frame.rows = rows;
    frame.dirtyRows = new Uint8Array(rows);
    frame.glyph = new Uint32Array(cells);
    frame.fg = new Uint32Array(cells);
    frame.bg = new Uint32Array(cells);
    frame.underlineColor = new Uint32Array(cells);
    frame.flags = new Uint16Array(cells);
    frame.selectionStart = new Int32Array(rows).fill(NO_SELECTION);
    frame.selectionEnd = new Int32Array(rows);
  }

  /** Read the default colours and the palette. Returns whether any of them changed. */
  #readColors(): boolean {
    const core = this.#core;
    const k = this.#k;
    core.x.ghostty_sized_init(this.#colors, k.colorsSize);
    this.#get(k.dataColors, this.#colors);

    const bytes = core.u8().subarray(this.#colors, this.#colors + k.colorsSize);
    const last = this.#lastColors;
    let same = true;
    for (let i = 0; i < bytes.length; i++) {
      if (bytes[i] !== last[i]) {
        same = false;
        break;
      }
    }
    if (same) return false;
    last.set(bytes);

    const color = (at: number) => rgba(bytes[at]!, bytes[at + 1]!, bytes[at + 2]!);
    const frame = this.frame;
    frame.background = color(k.colorsBackground);
    frame.foreground = color(k.colorsForeground);
    frame.cursor.hasColor = bytes[k.colorsCursorHasValue] !== 0;
    frame.cursor.color = frame.cursor.hasColor ? color(k.colorsCursor) : 0;
    for (let i = 0; i < 256; i++) this.#palette[i] = color(k.colorsPalette + i * 3);
    return true;
  }

  /** Read the cursor. Returns whether it changed. */
  #readCursor(): boolean {
    const core = this.#core;
    const k = this.#k;
    core.x.ghostty_sized_init(this.#cursor, k.cursorSize);
    this.#get(k.dataCursor, this.#cursor);

    const view = core.view();
    const at = this.#cursor;
    const inViewport = view.getUint8(at + k.cursorHasValue) !== 0;
    const visible = inViewport && view.getUint8(at + k.cursorVisible) !== 0;
    // The position is meaningful only while the cursor is inside the viewport.
    const cx = inViewport ? view.getUint16(at + k.cursorX, true) : 0;
    const cy = inViewport ? view.getUint16(at + k.cursorY, true) : 0;
    const wideTail = inViewport && view.getUint8(at + k.cursorWideTail) !== 0;
    const blinking = view.getUint8(at + k.cursorBlinking) !== 0;
    const shape = k.cursorShapes.get(view.getInt32(at + k.cursorStyle, true)) ?? CursorShape.Block;

    const cursor = this.frame.cursor;
    const changed =
      cursor.visible !== visible ||
      cursor.x !== cx ||
      cursor.y !== cy ||
      cursor.wideTail !== wideTail ||
      cursor.blinking !== blinking ||
      cursor.shape !== shape;
    cursor.visible = visible;
    cursor.x = cx;
    cursor.y = cy;
    cursor.wideTail = wideTail;
    cursor.blinking = blinking;
    cursor.shape = shape;
    return changed;
  }

  /** Decode the row the iterator is on into row `y` of the frame. */
  #readRow(y: number): void {
    const core = this.#core;
    const { x } = core;
    const k = this.#k;
    const frame = this.frame;
    const cols = frame.cols;
    const start = y * cols;

    core.x.ghostty_sized_init(this.#selection, k.selectionSize);
    const selected = x.ghostty_render_state_row_get(
      this.#iterator,
      k.rowSelection,
      this.#selection,
    );
    if (selected === NO_VALUE) {
      frame.selectionStart[y] = NO_SELECTION;
    } else {
      core.check(selected, "ghostty_render_state_row_get");
      const view = core.view();
      frame.selectionStart[y] = view.getUint16(this.#selection + k.selectionStartX, true);
      frame.selectionEnd[y] = view.getUint16(this.#selection + k.selectionEndX, true);
    }

    core.check(
      x.ghostty_render_state_row_get(this.#iterator, k.rowCellsRaw, this.#cellsView),
      "ghostty_render_state_row_get",
    );
    const view = core.view();
    const ptr = view.getUint32(this.#cellsView + k.cellsViewPtr, true);
    const count = Math.min(view.getUint32(this.#cellsView + k.cellsViewLen, true), cols);
    // Packed cells are 64-bit values, so the row is aligned for reading as words.
    if ((ptr & 3) !== 0) throw new Error("libghostty-vt returned a misaligned row of cells");
    const base = ptr >>> 2;

    const { glyph, fg, bg, underlineColor, flags } = frame;
    const defaultFg = frame.foreground;
    const defaultBg = frame.background;
    this.#styleSlots.clear();
    let styleCount = 0;
    let cellsReady = false;
    // Neighbouring cells usually share a style, so the last one is kept at hand.
    let lastId = -1;
    let lastSlot = -1;

    for (let col = 0; col < count; col++) {
      // Fetching a style or a cluster below calls into the module, which may replace its
      // memory. Taking the view again is a comparison when it has not.
      const words = core.u32();
      const lo = words[base + col * 2]!;
      const hi = words[base + col * 2 + 1]!;
      const index = start + col;

      const wide = k.wide(lo, hi);
      if (wide === k.wideSpacerTail && col > 0) {
        // The second half of a double-width character is drawn as part of the first.
        glyph[index] = 0;
        fg[index] = fg[index - 1]!;
        bg[index] = bg[index - 1]!;
        underlineColor[index] = underlineColor[index - 1]!;
        flags[index] = (flags[index - 1]! & DECORATIONS) | CellFlag.Spacer;
        continue;
      }

      let slot = -1;
      const id = k.styleId(lo, hi);
      if (id !== 0) {
        if (id === lastId) {
          slot = lastSlot;
        } else {
          let found = this.#styleSlots.get(id);
          if (found === undefined) {
            if (!cellsReady) {
              core.check(
                x.ghostty_render_state_row_get(this.#iterator, k.rowCells, this.#cellsSlot),
                "ghostty_render_state_row_get",
              );
              cellsReady = true;
            }
            found = styleCount++;
            this.#fetchStyle(col, found);
            this.#styleSlots.set(id, found);
          }
          slot = found;
          lastId = id;
          lastSlot = slot;
        }
      }
      const bits = slot < 0 ? 0 : this.#styleBits[slot]!;

      const tag = k.contentTag(lo, hi);
      const content = k.content(lo, hi);
      let cellGlyph = 0;
      let cellBg = (bits & STYLE_BG_SET) !== 0 ? this.#styleBg[slot]! : 0;
      if (tag === k.tagCodepoint) {
        cellGlyph = content & k.codepointMask;
      } else if (tag === k.tagGrapheme) {
        if (!cellsReady) {
          core.check(
            x.ghostty_render_state_row_get(this.#iterator, k.rowCells, this.#cellsSlot),
            "ghostty_render_state_row_get",
          );
          cellsReady = true;
        }
        cellGlyph = this.#cluster(col);
      } else if (tag === k.tagBgPalette) {
        // A cell erased with a background colour carries the colour in place of text.
        cellBg = this.#palette[content & 0xff]!;
      } else if (tag === k.tagBgRgb) {
        cellBg = rgba(content & 0xff, (content >>> 8) & 0xff, (content >>> 16) & 0xff);
      }

      let cellFg = (bits & STYLE_FG_SET) !== 0 ? this.#styleFg[slot]! : defaultFg;
      if ((bits & STYLE_INVERSE) !== 0) {
        const swapped = cellBg === 0 ? defaultBg : cellBg;
        cellBg = cellFg;
        cellFg = swapped;
      }
      if ((bits & STYLE_FAINT) !== 0) {
        cellFg = mix(cellFg, cellBg === 0 ? defaultBg : cellBg, FAINT_OPACITY);
      }

      let cellFlags = bits & FRAME_FLAGS;
      if (wide === k.wideWide) cellFlags |= CellFlag.Wide;
      if (k.hyperlink(lo, hi) !== 0) cellFlags |= CellFlag.Hyperlink;
      if ((bits & STYLE_INVISIBLE) !== 0) {
        // Concealed text keeps its background and nothing else.
        cellGlyph = 0;
        cellFlags &= ~DECORATIONS;
      }

      glyph[index] = cellGlyph;
      fg[index] = cellFg;
      bg[index] = cellBg;
      underlineColor[index] =
        (bits & STYLE_UNDERLINE_SET) !== 0 ? this.#styleUnderline[slot]! : cellFg;
      flags[index] = cellFlags;
    }

    for (let col = count; col < cols; col++) {
      const index = start + col;
      glyph[index] = 0;
      fg[index] = defaultFg;
      bg[index] = 0;
      underlineColor[index] = defaultFg;
      flags[index] = 0;
    }
  }

  /** Fetch the style of the cell at `col` of the current row into `slot`. */
  #fetchStyle(col: number, slot: number): void {
    const core = this.#core;
    const { x } = core;
    const k = this.#k;
    core.check(x.ghostty_render_state_row_cells_select(this.#cells, col), "row_cells_select");
    core.check(
      x.ghostty_render_state_row_cells_get(this.#cells, k.cellStyle, this.#style),
      "ghostty_render_state_row_cells_get",
    );

    if (slot >= this.#styleBits.length) {
      const grow = <T extends Uint32Array | Uint16Array>(from: T): T => {
        const to = new (from.constructor as new (n: number) => T)(from.length * 2);
        to.set(from);
        return to;
      };
      this.#styleFg = grow(this.#styleFg);
      this.#styleBg = grow(this.#styleBg);
      this.#styleUnderline = grow(this.#styleUnderline);
      this.#styleBits = grow(this.#styleBits);
    }

    const bytes = core.u8();
    const raw = bytes[this.#style + k.styleFlags]! | (bytes[this.#style + k.styleFlags + 1]! << 8);
    let bits = 0;
    if (k.flagBold(raw, 0) !== 0) bits |= CellFlag.Bold;
    if (k.flagItalic(raw, 0) !== 0) bits |= CellFlag.Italic;
    if (k.flagStrikethrough(raw, 0) !== 0) bits |= CellFlag.Strikethrough;
    if (k.flagOverline(raw, 0) !== 0) bits |= CellFlag.Overline;
    if (k.flagInverse(raw, 0) !== 0) bits |= STYLE_INVERSE;
    if (k.flagFaint(raw, 0) !== 0) bits |= STYLE_FAINT;
    if (k.flagInvisible(raw, 0) !== 0) bits |= STYLE_INVISIBLE;
    bits |= (k.underlines.get(k.flagUnderline(raw, 0)) ?? UnderlineStyle.Single) << UNDERLINE_SHIFT;

    const color = (at: number, set: number): number => {
      const tag = bytes[at + k.colorTag]!;
      const value = at + k.colorValue;
      if (tag === k.colorPalette) {
        bits |= set;
        return this.#palette[bytes[value]!]!;
      }
      if (tag === k.colorRgb) {
        bits |= set;
        return rgba(bytes[value]!, bytes[value + 1]!, bytes[value + 2]!);
      }
      return 0;
    };
    this.#styleFg[slot] = color(this.#style + k.styleFg, STYLE_FG_SET);
    this.#styleBg[slot] = color(this.#style + k.styleBg, STYLE_BG_SET);
    this.#styleUnderline[slot] = color(this.#style + k.styleUnderline, STYLE_UNDERLINE_SET);
    this.#styleBits[slot] = bits;
  }

  /** The `glyph` entry for the multi-codepoint cell at `col` of the current row. */
  #cluster(col: number): number {
    const core = this.#core;
    const { x } = core;
    const k = this.#k;
    core.check(x.ghostty_render_state_row_cells_select(this.#cells, col), "row_cells_select");
    let view = core.view();
    view.setUint32(this.#codepointBuffer + k.bufferPtr, this.#codepoints, true);
    view.setUint32(this.#codepointBuffer + k.bufferCap, GRAPHEME_CAP, true);
    view.setUint32(this.#codepointBuffer + k.bufferLen, 0, true);
    const result = x.ghostty_render_state_row_cells_get(
      this.#cells,
      k.cellGraphemes,
      this.#codepointBuffer,
    );
    // Out of space still leaves the first codepoints in the buffer, which is enough to
    // draw something for a cluster longer than any real one.
    if (result !== OUT_OF_SPACE) core.check(result, "ghostty_render_state_row_cells_get");

    view = core.view();
    const count = Math.min(view.getUint32(this.#codepointBuffer + k.bufferLen, true), GRAPHEME_CAP);
    if (count === 0) return 0;
    if (count === 1) return view.getUint32(this.#codepoints, true);
    let text = "";
    for (let i = 0; i < count; i++) {
      const codepoint = view.getUint32(this.#codepoints + i * 4, true);
      if (codepoint <= 0x10ffff && (codepoint < 0xd800 || codepoint > 0xdfff)) {
        text += String.fromCodePoint(codepoint);
      }
    }
    let index = this.#clusterIndex.get(text);
    if (index === undefined) {
      index = this.#clusters.length;
      this.#clusters.push(text);
      this.#clusterIndex.set(text, index);
    }
    return (GLYPH_CLUSTER_TAG | index) >>> 0;
  }

  #get(key: number, out: number): void {
    this.#core.check(
      this.#core.x.ghostty_render_state_get(this.#state, key, out),
      "ghostty_render_state_get",
    );
  }

  #getU16(key: number): number {
    this.#get(key, this.#out);
    return this.#core.view().getUint16(this.#out, true);
  }
}

/** `a` over `b` with `a` at the given opacity, per channel. The result is opaque. */
function mix(a: number, b: number, opacity: number): number {
  const channel = (shift: number) => {
    const from = (a >>> shift) & 0xff;
    const to = (b >>> shift) & 0xff;
    return Math.round(from * opacity + to * (1 - opacity));
  };
  return rgba(channel(0), channel(8), channel(16));
}

type Bits = (lo: number, hi: number) => number;

/** Everything `RenderState` reads from the type manifest, looked up once per module. */
interface Keys {
  dataCols: number;
  dataRows: number;
  dataDirty: number;
  dataRowIterator: number;
  dataCursor: number;
  dataColors: number;
  dirtyFalse: number;
  dirtyFull: number;
  rowCells: number;
  rowSelection: number;
  rowCellsRaw: number;
  cellStyle: number;
  cellGraphemes: number;

  cellsViewSize: number;
  cellsViewPtr: number;
  cellsViewLen: number;
  selectionSize: number;
  selectionStartX: number;
  selectionEndX: number;
  cursorSize: number;
  cursorHasValue: number;
  cursorX: number;
  cursorY: number;
  cursorWideTail: number;
  cursorVisible: number;
  cursorBlinking: number;
  cursorStyle: number;
  cursorShapes: ReadonlyMap<number, number>;
  colorsSize: number;
  colorsBackground: number;
  colorsForeground: number;
  colorsCursor: number;
  colorsCursorHasValue: number;
  colorsPalette: number;
  codepointBufferSize: number;
  bufferPtr: number;
  bufferCap: number;
  bufferLen: number;

  styleSize: number;
  styleFg: number;
  styleBg: number;
  styleUnderline: number;
  styleFlags: number;
  colorTag: number;
  colorValue: number;
  colorPalette: number;
  colorRgb: number;
  flagBold: Bits;
  flagItalic: Bits;
  flagFaint: Bits;
  flagInverse: Bits;
  flagInvisible: Bits;
  flagStrikethrough: Bits;
  flagOverline: Bits;
  flagUnderline: Bits;
  underlines: ReadonlyMap<number, number>;

  contentTag: Bits;
  content: Bits;
  styleId: Bits;
  wide: Bits;
  hyperlink: Bits;
  codepointMask: number;
  tagCodepoint: number;
  tagGrapheme: number;
  tagBgPalette: number;
  tagBgRgb: number;
  wideWide: number;
  wideSpacerTail: number;
}

const keysByCore = new WeakMap<Core, Keys>();

function keys(core: Core): Keys {
  const existing = keysByCore.get(core);
  if (existing) return existing;
  const m = core.manifest;
  const data = (name: string) => m.enumValue("GhosttyRenderStateData", name);
  const row = (name: string) => m.enumValue("GhosttyRenderStateRowData", name);
  const cell = (name: string) => m.enumValue("GhosttyRenderStateRowCellsData", name);
  const flag = (name: string) => bitReader(m.bits("GhosttyStyleFlags", name));
  const bits = (name: string) => bitReader(m.bits("GhosttyCell", name));

  // The decoder reads a cell's content as one field and picks it apart by tag, which
  // holds only while each arm starts at the bottom of that field in the layout below.
  const codepoint = m.armBits("GhosttyCell", "content", "CODEPOINT", "codepoint");
  const grapheme = m.armBits("GhosttyCell", "content", "CODEPOINT_GRAPHEME", "codepoint");
  const index = m.armBits("GhosttyCell", "content", "BG_COLOR_PALETTE", "index");
  const rgb = (["r", "g", "b"] as const).map((name) =>
    m.armBits("GhosttyCell", "content", "BG_COLOR_RGB", name),
  );
  const expected =
    codepoint.lsb === 0 &&
    grapheme.lsb === 0 &&
    grapheme.width === codepoint.width &&
    index.lsb === 0 &&
    index.width === 8 &&
    rgb.every((field, i) => field.lsb === i * 8 && field.width === 8);
  if (!expected) {
    throw new Error("libghostty-vt packs cell content in a layout this wrapper does not read");
  }

  const cursorStyle = (name: string) => m.enumValue("GhosttyCursorStyle", name);
  const underline = (name: string) => m.enumValue("GhosttySgrUnderline", name);
  const created: Keys = {
    dataCols: data("COLS"),
    dataRows: data("ROWS"),
    dataDirty: data("DIRTY"),
    dataRowIterator: data("ROW_ITERATOR"),
    dataCursor: data("CURSOR"),
    dataColors: data("COLORS"),
    dirtyFalse: m.enumValue("GhosttyRenderStateDirty", "FALSE"),
    dirtyFull: m.enumValue("GhosttyRenderStateDirty", "FULL"),
    rowCells: row("CELLS"),
    rowSelection: row("SELECTION"),
    rowCellsRaw: row("CELLS_RAW"),
    cellStyle: cell("STYLE"),
    cellGraphemes: cell("GRAPHEMES_BUF"),

    cellsViewSize: m.struct("GhosttyCellsView").size,
    cellsViewPtr: m.offset("GhosttyCellsView", "ptr"),
    cellsViewLen: m.offset("GhosttyCellsView", "len"),
    selectionSize: m.struct("GhosttyRenderStateRowSelection").size,
    selectionStartX: m.offset("GhosttyRenderStateRowSelection", "start_x"),
    selectionEndX: m.offset("GhosttyRenderStateRowSelection", "end_x"),
    cursorSize: m.struct("GhosttyRenderStateCursor").size,
    cursorHasValue: m.offset("GhosttyRenderStateCursor", "viewport_has_value"),
    cursorX: m.offset("GhosttyRenderStateCursor", "viewport_x"),
    cursorY: m.offset("GhosttyRenderStateCursor", "viewport_y"),
    cursorWideTail: m.offset("GhosttyRenderStateCursor", "wide_tail"),
    cursorVisible: m.offset("GhosttyRenderStateCursor", "visible"),
    cursorBlinking: m.offset("GhosttyRenderStateCursor", "blinking"),
    cursorStyle: m.offset("GhosttyRenderStateCursor", "visual_style"),
    cursorShapes: new Map([
      [cursorStyle("BAR"), CursorShape.Bar],
      [cursorStyle("BLOCK"), CursorShape.Block],
      [cursorStyle("UNDERLINE"), CursorShape.Underline],
      [cursorStyle("BLOCK_HOLLOW"), CursorShape.HollowBlock],
    ]),
    colorsSize: m.struct("GhosttyRenderStateColors").size,
    colorsBackground: m.offset("GhosttyRenderStateColors", "background"),
    colorsForeground: m.offset("GhosttyRenderStateColors", "foreground"),
    colorsCursor: m.offset("GhosttyRenderStateColors", "cursor"),
    colorsCursorHasValue: m.offset("GhosttyRenderStateColors", "cursor_has_value"),
    colorsPalette: m.offset("GhosttyRenderStateColors", "palette"),
    codepointBufferSize: m.struct("GhosttyCodepointBuffer").size,
    bufferPtr: m.offset("GhosttyCodepointBuffer", "ptr"),
    bufferCap: m.offset("GhosttyCodepointBuffer", "cap"),
    bufferLen: m.offset("GhosttyCodepointBuffer", "len"),

    styleSize: m.struct("GhosttyStyle").size,
    styleFg: m.offset("GhosttyStyle", "fg_color"),
    styleBg: m.offset("GhosttyStyle", "bg_color"),
    styleUnderline: m.offset("GhosttyStyle", "underline_color"),
    styleFlags: m.offset("GhosttyStyle", "flags"),
    colorTag: m.offset("GhosttyStyleColor", "tag"),
    colorValue: m.offset("GhosttyStyleColor", "value"),
    colorPalette: m.enumValue("GhosttyStyleColorTag", "PALETTE"),
    colorRgb: m.enumValue("GhosttyStyleColorTag", "RGB"),
    flagBold: flag("bold"),
    flagItalic: flag("italic"),
    flagFaint: flag("faint"),
    flagInverse: flag("inverse"),
    flagInvisible: flag("invisible"),
    flagStrikethrough: flag("strikethrough"),
    flagOverline: flag("overline"),
    flagUnderline: flag("underline"),
    underlines: new Map([
      [underline("NONE"), UnderlineStyle.None],
      [underline("SINGLE"), UnderlineStyle.Single],
      [underline("DOUBLE"), UnderlineStyle.Double],
      [underline("CURLY"), UnderlineStyle.Curly],
      [underline("DOTTED"), UnderlineStyle.Dotted],
      [underline("DASHED"), UnderlineStyle.Dashed],
    ]),

    contentTag: bits("content_tag"),
    content: bits("content"),
    styleId: bits("style_id"),
    wide: bits("wide"),
    hyperlink: bits("hyperlink"),
    codepointMask: 2 ** codepoint.width - 1,
    tagCodepoint: m.enumValue("GhosttyCellContentTag", "CODEPOINT"),
    tagGrapheme: m.enumValue("GhosttyCellContentTag", "CODEPOINT_GRAPHEME"),
    tagBgPalette: m.enumValue("GhosttyCellContentTag", "BG_COLOR_PALETTE"),
    tagBgRgb: m.enumValue("GhosttyCellContentTag", "BG_COLOR_RGB"),
    wideWide: m.enumValue("GhosttyCellWide", "WIDE"),
    wideSpacerTail: m.enumValue("GhosttyCellWide", "SPACER_TAIL"),
  };
  keysByCore.set(core, created);
  return created;
}
