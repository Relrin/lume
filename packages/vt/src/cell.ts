/**
 * Reading single cells through grid references.
 *
 * This is the slow, exact path: one cell at a time, with the style and text as the
 * terminal stores them and no colour resolution. It backs screen dumps and text capture.
 * Drawing uses the render state instead, which reads a row at a time.
 */

import { OUT_OF_SPACE } from "./error.ts";
import type { BitField } from "./manifest.ts";
import type { Core } from "./module.ts";

/** A colour as a program set it, before the palette is applied. */
export type Color =
  | { kind: "default" }
  | { kind: "palette"; index: number }
  | { kind: "rgb"; r: number; g: number; b: number };

/** How a cell takes part in a double-width character. */
export type Wide = "narrow" | "wide" | "spacer-tail" | "spacer-head";

/** The text attributes of a cell. */
export interface Attrs {
  bold: boolean;
  italic: boolean;
  faint: boolean;
  inverse: boolean;
  strikethrough: boolean;
  /** The underline style number libghostty-vt reports; zero means none. */
  underline: number;
}

/** One cell of the grid. */
export interface Cell {
  /** The grapheme cluster in the cell. Empty for a blank cell. */
  text: string;
  fg: Color;
  bg: Color;
  attrs: Attrs;
  wide: Wide;
}

/** Which rows a point's `row` counts from. */
export type PointSpace = "active" | "viewport";

/** Codepoints read per cell before falling back to a larger buffer. */
const GRAPHEME_BUF_LEN = 16;

/** Bytes read for a hyperlink target before falling back to a larger buffer. */
const URI_BUF_LEN = 512;

const readers = new WeakMap<Core, CellReader>();

/** The cell reader of a module. Its scratch memory is shared by every terminal. */
export function cellReader(core: Core): CellReader {
  let reader = readers.get(core);
  if (!reader) {
    reader = new CellReader(core);
    readers.set(core, reader);
  }
  return reader;
}

/** Reads cells of a terminal through grid references. */
export class CellReader {
  readonly #core: Core;

  // Scratch memory inside the module, allocated once.
  readonly #point: number;
  readonly #pointSize: number;
  readonly #gridRef: number;
  readonly #style: number;
  readonly #out: number;
  readonly #graphemes: number;
  readonly #uri: number;
  readonly #written: number;

  // Offsets and values, read from the module's own description of its types.
  readonly #pointTagAt: number;
  readonly #pointXAt: number;
  readonly #pointYAt: number;
  readonly #tagActive: number;
  readonly #tagViewport: number;
  readonly #cellDataWide: number;
  readonly #wideNames: ReadonlyMap<number, Wide>;
  readonly #fgAt: number;
  readonly #bgAt: number;
  readonly #flagsAt: number;
  readonly #colorTagAt: number;
  readonly #colorValueAt: number;
  readonly #colorPalette: number;
  readonly #colorRgb: number;
  readonly #flag: Readonly<Record<keyof Attrs, BitField>>;

  constructor(core: Core) {
    this.#core = core;
    const m = core.manifest;

    const point = m.struct("GhosttyPoint");
    const style = m.struct("GhosttyStyle");
    const align8 = (n: number) => (n + 7) & ~7;
    // One allocation in 8-byte slots. The allocator aligns the block to the largest
    // alignment of the ABI, so every slot is aligned for whatever is placed in it.
    const sizes = [
      point.size,
      m.struct("GhosttyGridRef").size,
      style.size,
      8,
      GRAPHEME_BUF_LEN * 4,
      URI_BUF_LEN,
      4,
    ];
    let offset = 0;
    const base = core.allocZeroed(sizes.reduce((sum, size) => sum + align8(size), 0));
    const [pointAt, gridRefAt, styleAt, outAt, graphemesAt, uriAt, writtenAt] = sizes.map(
      (size) => {
        const at = base + offset;
        offset += align8(size);
        return at;
      },
    ) as [number, number, number, number, number, number, number];
    this.#point = pointAt;
    this.#pointSize = point.size;
    this.#gridRef = gridRefAt;
    this.#style = styleAt;
    this.#out = outAt;
    this.#graphemes = graphemesAt;
    this.#uri = uriAt;
    this.#written = writtenAt;

    this.#pointTagAt = m.offset("GhosttyPoint", "tag");
    const coordinate = m.offset("GhosttyPoint", "value");
    this.#pointXAt = coordinate + m.offset("GhosttyPointCoordinate", "x");
    this.#pointYAt = coordinate + m.offset("GhosttyPointCoordinate", "y");
    this.#tagActive = m.enumValue("GhosttyPointTag", "ACTIVE");
    this.#tagViewport = m.enumValue("GhosttyPointTag", "VIEWPORT");

    this.#cellDataWide = m.enumValue("GhosttyCellData", "WIDE");
    this.#wideNames = new Map<number, Wide>([
      [m.enumValue("GhosttyCellWide", "WIDE"), "wide"],
      [m.enumValue("GhosttyCellWide", "SPACER_TAIL"), "spacer-tail"],
      [m.enumValue("GhosttyCellWide", "SPACER_HEAD"), "spacer-head"],
    ]);

    this.#fgAt = m.offset("GhosttyStyle", "fg_color");
    this.#bgAt = m.offset("GhosttyStyle", "bg_color");
    this.#flagsAt = m.offset("GhosttyStyle", "flags");
    this.#colorTagAt = m.offset("GhosttyStyleColor", "tag");
    this.#colorValueAt = m.offset("GhosttyStyleColor", "value");
    this.#colorPalette = m.enumValue("GhosttyStyleColorTag", "PALETTE");
    this.#colorRgb = m.enumValue("GhosttyStyleColorTag", "RGB");
    this.#flag = {
      bold: m.bits("GhosttyStyleFlags", "bold"),
      italic: m.bits("GhosttyStyleFlags", "italic"),
      faint: m.bits("GhosttyStyleFlags", "faint"),
      inverse: m.bits("GhosttyStyleFlags", "inverse"),
      strikethrough: m.bits("GhosttyStyleFlags", "strikethrough"),
      underline: m.bits("GhosttyStyleFlags", "underline"),
    };
  }

  /**
   * Resolve a grid reference to the cell at `col`, `row`.
   *
   * Returns the address of the reference. It lives in scratch memory and is good only
   * until the next call on this reader or the next change to the terminal.
   */
  ref(terminal: number, space: PointSpace, col: number, row: number): number {
    const { x } = this.#core;
    // A point is passed by value in C; the export takes the address of a copy.
    this.#core.u8().fill(0, this.#point, this.#point + this.#pointSize);
    const view = this.#core.view();
    const tag = space === "active" ? this.#tagActive : this.#tagViewport;
    view.setInt32(this.#point + this.#pointTagAt, tag, true);
    view.setUint16(this.#point + this.#pointXAt, col, true);
    view.setUint32(this.#point + this.#pointYAt, row, true);
    this.#core.check(
      x.ghostty_terminal_grid_ref(terminal, this.#point, this.#gridRef),
      "ghostty_terminal_grid_ref",
    );
    return this.#gridRef;
  }

  /** The cell at `col`, `row` of the active area. */
  read(terminal: number, col: number, row: number): Cell {
    const core = this.#core;
    const { x } = core;
    this.ref(terminal, "active", col, row);

    core.check(x.ghostty_grid_ref_cell(this.#gridRef, this.#out), "ghostty_grid_ref_cell");
    const raw = core.view().getBigUint64(this.#out, true);
    core.check(x.ghostty_cell_get(raw, this.#cellDataWide, this.#out), "ghostty_cell_get");
    const wide = this.#wideNames.get(core.view().getInt32(this.#out, true)) ?? "narrow";

    core.check(x.ghostty_grid_ref_style(this.#gridRef, this.#style), "ghostty_grid_ref_style");
    const view = core.view();
    const flags = view.getUint16(this.#style + this.#flagsAt, true);
    const bit = (field: BitField) => (flags >>> field.lsb) & (2 ** field.width - 1);
    const fg = this.#color(view, this.#style + this.#fgAt);
    const bg = this.#color(view, this.#style + this.#bgAt);

    return {
      text: this.#graphemeText(),
      fg,
      bg,
      attrs: {
        bold: bit(this.#flag.bold) !== 0,
        italic: bit(this.#flag.italic) !== 0,
        faint: bit(this.#flag.faint) !== 0,
        inverse: bit(this.#flag.inverse) !== 0,
        strikethrough: bit(this.#flag.strikethrough) !== 0,
        underline: bit(this.#flag.underline),
      },
      wide,
    };
  }

  /** The target of the hyperlink on the cell `ref()` last resolved, or `""` for none. */
  hyperlinkUri(): string {
    const core = this.#core;
    const { x } = core;
    let buf = this.#uri;
    let len = URI_BUF_LEN;
    let result = x.ghostty_grid_ref_hyperlink_uri(this.#gridRef, buf, len, this.#written);
    let big = 0;
    if (result === OUT_OF_SPACE) {
      // `written` holds the length the target needs.
      len = core.view().getUint32(this.#written, true);
      big = core.alloc(len);
      buf = big;
      result = x.ghostty_grid_ref_hyperlink_uri(this.#gridRef, buf, len, this.#written);
    }
    try {
      core.check(result, "ghostty_grid_ref_hyperlink_uri");
      return core.string(buf, core.view().getUint32(this.#written, true));
    } finally {
      if (big !== 0) core.free(big, len);
    }
  }

  #color(view: DataView, at: number): Color {
    const tag = view.getUint8(at + this.#colorTagAt);
    const value = at + this.#colorValueAt;
    if (tag === this.#colorPalette) return { kind: "palette", index: view.getUint8(value) };
    if (tag === this.#colorRgb) {
      return {
        kind: "rgb",
        r: view.getUint8(value),
        g: view.getUint8(value + 1),
        b: view.getUint8(value + 2),
      };
    }
    return { kind: "default" };
  }

  /** The text of the cell `#gridRef` points at. */
  #graphemeText(): string {
    const core = this.#core;
    const { x } = core;
    let buf = this.#graphemes;
    let len = GRAPHEME_BUF_LEN;
    let result = x.ghostty_grid_ref_graphemes(this.#gridRef, buf, len, this.#written);
    let big = 0;
    if (result === OUT_OF_SPACE) {
      // `written` holds the number of codepoints. Clusters this long are rare.
      len = core.view().getUint32(this.#written, true);
      big = core.alloc(len * 4);
      buf = big;
      result = x.ghostty_grid_ref_graphemes(this.#gridRef, buf, len, this.#written);
    }
    try {
      core.check(result, "ghostty_grid_ref_graphemes");
      const view = core.view();
      const count = view.getUint32(this.#written, true);
      let text = "";
      for (let i = 0; i < count; i++) {
        const codepoint = view.getUint32(buf + i * 4, true);
        if (codepoint !== 0 && isScalarValue(codepoint)) text += String.fromCodePoint(codepoint);
      }
      return text;
    } finally {
      if (big !== 0) core.free(big, len * 4);
    }
  }
}

/** Whether a number is a Unicode scalar value: in range and not a surrogate. */
export function isScalarValue(codepoint: number): boolean {
  return codepoint <= 0x10ffff && (codepoint < 0xd800 || codepoint > 0xdfff);
}
