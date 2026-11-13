/**
 * A TypeScript wrapper over the WebAssembly build of libghostty-vt.
 *
 * Current scope: create a terminal, write bytes, resize, and read back text and cell
 * attributes. It mirrors the Rust `lume-vt` crate so both builds can be checked against
 * the same expectations. Render state, input encoding and selection are not wrapped yet.
 *
 * The wrapper uses only functions the unpatched upstream module exports. Struct layouts
 * are not hard-coded: they are read from the module's own `ghostty_type_json()`.
 */

/** The exports this wrapper calls. Pointers are numbers (wasm32); `GhosttyCell` is 64-bit. */
interface Exports {
  memory: WebAssembly.Memory;
  ghostty_wasm_alloc(len: number): number;
  ghostty_wasm_free(ptr: number, len: number): void;
  ghostty_wasm_alloc_opaque(): number;
  ghostty_wasm_take_opaque(slot: number): number;
  ghostty_wasm_free_opaque(slot: number): void;
  ghostty_version(): number;
  ghostty_type_json(): number;
  ghostty_terminal_new(allocator: number, out: number, cols: number, rows: number): number;
  ghostty_terminal_free(terminal: number): void;
  ghostty_terminal_vt_write(terminal: number, data: number, len: number): void;
  ghostty_terminal_resize(
    terminal: number,
    cols: number,
    rows: number,
    cellWidthPx: number,
    cellHeightPx: number,
  ): number;
  ghostty_terminal_get(terminal: number, key: number, out: number): number;
  ghostty_terminal_grid_ref(terminal: number, point: number, out: number): number;
  ghostty_grid_ref_cell(ref: number, out: number): number;
  ghostty_grid_ref_style(ref: number, out: number): number;
  ghostty_grid_ref_graphemes(ref: number, buf: number, bufLen: number, outWritten: number): number;
  ghostty_cell_get(cell: bigint, key: number, out: number): number;
}

const REQUIRED_EXPORTS: readonly (keyof Exports)[] = [
  "memory",
  "ghostty_wasm_alloc",
  "ghostty_wasm_free",
  "ghostty_wasm_alloc_opaque",
  "ghostty_wasm_take_opaque",
  "ghostty_wasm_free_opaque",
  "ghostty_version",
  "ghostty_type_json",
  "ghostty_terminal_new",
  "ghostty_terminal_free",
  "ghostty_terminal_vt_write",
  "ghostty_terminal_resize",
  "ghostty_terminal_get",
  "ghostty_terminal_grid_ref",
  "ghostty_grid_ref_cell",
  "ghostty_grid_ref_style",
  "ghostty_grid_ref_graphemes",
  "ghostty_cell_get",
];

// Values from the C headers of the pinned release (ghostty/vt/*.h).
const GHOSTTY_SUCCESS = 0;
const GHOSTTY_OUT_OF_SPACE = -3;
const POINT_TAG_ACTIVE = 0;
const CELL_DATA_WIDE = 3;
const CELL_WIDE_WIDE = 1;
const CELL_WIDE_SPACER_TAIL = 2;
const CELL_WIDE_SPACER_HEAD = 3;
const STYLE_COLOR_PALETTE = 1;
const STYLE_COLOR_RGB = 2;
const STYLE_UNDERLINE_SHIFT = 8;
const TERMINAL_DATA_CURSOR_X = 3;
const TERMINAL_DATA_CURSOR_Y = 4;
const TERMINAL_DATA_CURSOR_PENDING_WRAP = 5;
const TERMINAL_DATA_ACTIVE_SCREEN = 6;
const TERMINAL_DATA_CURSOR_VISIBLE = 7;
const TERMINAL_SCREEN_ALTERNATE = 1;

const GRAPHEME_BUF_LEN = 16;

export class VtError extends Error {
  constructor(
    readonly code: number,
    what: string,
  ) {
    super(`libghostty-vt: ${what} failed with code ${code}`);
    this.name = "VtError";
  }
}

export type Color =
  | { kind: "default" }
  | { kind: "palette"; index: number }
  | { kind: "rgb"; r: number; g: number; b: number };

export type Wide = "narrow" | "wide" | "spacer-tail" | "spacer-head";

export interface Attrs {
  bold: boolean;
  italic: boolean;
  faint: boolean;
  inverse: boolean;
  strikethrough: boolean;
  /** The underline style number libghostty-vt reports; zero means none. */
  underline: number;
}

export interface Cell {
  /** The grapheme cluster in the cell. Empty for a blank cell. */
  text: string;
  fg: Color;
  bg: Color;
  attrs: Attrs;
  wide: Wide;
}

export interface Cursor {
  x: number;
  y: number;
  visible: boolean;
  pendingWrap: boolean;
}

interface FieldLayout {
  offset: number;
  size: number;
}
interface StructLayout {
  size: number;
  fields: Record<string, FieldLayout>;
}

/** The struct layouts this wrapper needs, as the module reports them for its own ABI. */
interface Layouts {
  point: StructLayout;
  pointCoordinate: StructLayout;
  gridRef: StructLayout;
  style: StructLayout;
  styleColor: StructLayout;
}

/** An instantiated libghostty-vt module. One per page is enough; terminals share it. */
export class Vt {
  /** The library version the module reports, for example `"1.0.1"`. */
  readonly version: string;
  readonly #exports: Exports;
  readonly #layouts: Layouts;

  private constructor(exports: Exports) {
    this.#exports = exports;
    this.version = this.#cString(exports.ghostty_version());

    const types = (
      JSON.parse(this.#cString(exports.ghostty_type_json())) as {
        types: Record<string, StructLayout | undefined>;
      }
    ).types;
    const layout = (name: string): StructLayout => {
      const found = types[name];
      if (!found?.fields) throw new Error(`libghostty-vt does not describe ${name}`);
      return found;
    };
    this.#layouts = {
      point: layout("GhosttyPoint"),
      pointCoordinate: layout("GhosttyPointCoordinate"),
      gridRef: layout("GhosttyGridRef"),
      style: layout("GhosttyStyle"),
      styleColor: layout("GhosttyStyleColor"),
    };
  }

  /** Instantiate the module from its bytes or from a fetch response. */
  static async instantiate(
    source: ArrayBuffer | Uint8Array | Response | Promise<Response>,
  ): Promise<Vt> {
    const isResponse = (s: unknown): s is Response | Promise<Response> =>
      (typeof Response !== "undefined" && s instanceof Response) || s instanceof Promise;

    // The module has no imports: it is freestanding.
    const { instance } = isResponse(source)
      ? await WebAssembly.instantiateStreaming(source, {})
      : await WebAssembly.instantiate(source as BufferSource, {});

    const exports = instance.exports as unknown as Exports;
    const missing = REQUIRED_EXPORTS.filter((name) => !(name in instance.exports));
    if (missing.length > 0) {
      throw new Error(`ghostty-vt.wasm lacks required exports: ${missing.join(", ")}`);
    }
    return new Vt(exports);
  }

  /** Create a terminal with the given grid size. Call `free()` on it when done. */
  createTerminal(cols: number, rows: number): Terminal {
    return new Terminal(this.#exports, this.#layouts, cols, rows);
  }

  #cString(ptr: number): string {
    const bytes = new Uint8Array(this.#exports.memory.buffer);
    let end = ptr;
    while (bytes[end] !== 0) end++;
    return new TextDecoder().decode(bytes.subarray(ptr, end));
  }
}

/** A terminal emulator: screen, scrollback, cursor, styles and modes. */
export class Terminal {
  readonly #x: Exports;
  readonly #layouts: Layouts;
  #handle: number;
  #cols: number;
  #rows: number;

  // Scratch memory inside the module, allocated once and reused for every call.
  readonly #scratch: number;
  readonly #scratchLen: number;
  readonly #point: number;
  readonly #gridRef: number;
  readonly #style: number;
  readonly #out: number;
  readonly #graphemes: number;
  readonly #written: number;

  // A reusable buffer for bytes passed to `write`.
  #input = 0;
  #inputCap = 0;

  /** @internal Use `Vt.createTerminal`. */
  constructor(exports: Exports, layouts: Layouts, cols: number, rows: number) {
    this.#x = exports;
    this.#layouts = layouts;
    this.#cols = cols;
    this.#rows = rows;

    // One allocation, laid out in 8-byte-aligned slots. The allocator aligns the block to
    // the ABI's maximum alignment, so every slot is aligned for any type placed in it.
    const align8 = (n: number) => (n + 7) & ~7;
    let offset = 0;
    const slot = (size: number) => {
      const at = offset;
      offset += align8(size);
      return at;
    };
    const pointAt = slot(layouts.point.size);
    const gridRefAt = slot(layouts.gridRef.size);
    const styleAt = slot(layouts.style.size);
    const outAt = slot(8);
    const graphemesAt = slot(GRAPHEME_BUF_LEN * 4);
    const writtenAt = slot(4);
    this.#scratchLen = offset;
    this.#scratch = exports.ghostty_wasm_alloc(this.#scratchLen);
    if (this.#scratch === 0) throw new VtError(-1, "ghostty_wasm_alloc");
    this.#point = this.#scratch + pointAt;
    this.#gridRef = this.#scratch + gridRefAt;
    this.#style = this.#scratch + styleAt;
    this.#out = this.#scratch + outAt;
    this.#graphemes = this.#scratch + graphemesAt;
    this.#written = this.#scratch + writtenAt;

    const handleSlot = exports.ghostty_wasm_alloc_opaque();
    if (handleSlot === 0) {
      exports.ghostty_wasm_free(this.#scratch, this.#scratchLen);
      throw new VtError(-1, "ghostty_wasm_alloc_opaque");
    }
    const result = exports.ghostty_terminal_new(0, handleSlot, cols, rows);
    this.#handle = result === GHOSTTY_SUCCESS ? exports.ghostty_wasm_take_opaque(handleSlot) : 0;
    exports.ghostty_wasm_free_opaque(handleSlot);
    if (this.#handle === 0) {
      exports.ghostty_wasm_free(this.#scratch, this.#scratchLen);
      throw new VtError(result, "ghostty_terminal_new");
    }
  }

  get cols(): number {
    return this.#cols;
  }

  get rows(): number {
    return this.#rows;
  }

  /** Feed terminal output (text and escape sequences) into the terminal. */
  write(data: Uint8Array | string): void {
    const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
    if (bytes.length === 0) return;
    this.#live();
    if (bytes.length > this.#inputCap) {
      if (this.#input !== 0) this.#x.ghostty_wasm_free(this.#input, this.#inputCap);
      this.#inputCap = Math.max(bytes.length, 64 * 1024);
      this.#input = this.#x.ghostty_wasm_alloc(this.#inputCap);
      if (this.#input === 0) {
        this.#inputCap = 0;
        throw new VtError(-1, "ghostty_wasm_alloc");
      }
    }
    // Take the view after allocating: an allocation may grow memory and detach old views.
    new Uint8Array(this.#x.memory.buffer).set(bytes, this.#input);
    this.#x.ghostty_terminal_vt_write(this.#handle, this.#input, bytes.length);
  }

  /** Resize the grid. Cell pixel sizes are passed as zero until the renderer supplies them. */
  resize(cols: number, rows: number): void {
    this.#live();
    this.#check(this.#x.ghostty_terminal_resize(this.#handle, cols, rows, 0, 0), "resize");
    this.#cols = cols;
    this.#rows = rows;
  }

  /** The cell at `col`, `row` of the active area (the screen without scrollback). */
  cell(col: number, row: number): Cell {
    this.#live();
    if (col < 0 || row < 0 || col >= this.#cols || row >= this.#rows) {
      throw new RangeError(`cell (${col}, ${row}) is outside ${this.#cols}x${this.#rows}`);
    }
    const x = this.#x;
    const { point, pointCoordinate, style, styleColor } = this.#layouts;

    // GhosttyPoint is passed by value in C; in the wasm ABI that is a pointer to a copy.
    let view = this.#view();
    new Uint8Array(x.memory.buffer, this.#point, point.size).fill(0);
    view.setInt32(this.#point + field(point, "tag").offset, POINT_TAG_ACTIVE, true);
    const coord = this.#point + field(point, "value").offset;
    view.setUint16(coord + field(pointCoordinate, "x").offset, col, true);
    view.setUint32(coord + field(pointCoordinate, "y").offset, row, true);
    new Uint8Array(x.memory.buffer, this.#gridRef, this.#layouts.gridRef.size).fill(0);
    this.#check(x.ghostty_terminal_grid_ref(this.#handle, this.#point, this.#gridRef), "grid_ref");

    this.#check(x.ghostty_grid_ref_cell(this.#gridRef, this.#out), "grid_ref_cell");
    const rawCell = this.#view().getBigUint64(this.#out, true);

    this.#check(x.ghostty_cell_get(rawCell, CELL_DATA_WIDE, this.#out), "cell_get");
    const wideRaw = this.#view().getInt32(this.#out, true);
    const wide: Wide =
      wideRaw === CELL_WIDE_WIDE
        ? "wide"
        : wideRaw === CELL_WIDE_SPACER_TAIL
          ? "spacer-tail"
          : wideRaw === CELL_WIDE_SPACER_HEAD
            ? "spacer-head"
            : "narrow";

    this.#check(x.ghostty_grid_ref_style(this.#gridRef, this.#style), "grid_ref_style");
    view = this.#view();
    const color = (name: string): Color => {
      const at = this.#style + field(style, name).offset;
      const tag = view.getUint8(at + field(styleColor, "tag").offset);
      const value = at + field(styleColor, "value").offset;
      if (tag === STYLE_COLOR_PALETTE) return { kind: "palette", index: view.getUint8(value) };
      if (tag === STYLE_COLOR_RGB) {
        return {
          kind: "rgb",
          r: view.getUint8(value),
          g: view.getUint8(value + 1),
          b: view.getUint8(value + 2),
        };
      }
      return { kind: "default" };
    };
    const flags = view.getUint16(this.#style + field(style, "flags").offset, true);
    const fg = color("fg_color");
    const bg = color("bg_color");

    return {
      text: this.#graphemeText(),
      fg,
      bg,
      attrs: {
        bold: (flags & (1 << 0)) !== 0,
        italic: (flags & (1 << 1)) !== 0,
        faint: (flags & (1 << 2)) !== 0,
        inverse: (flags & (1 << 4)) !== 0,
        strikethrough: (flags & (1 << 6)) !== 0,
        underline: (flags >> STYLE_UNDERLINE_SHIFT) & 0x7,
      },
      wide,
    };
  }

  /** The text of one row of the active area, with trailing blanks removed. */
  rowText(row: number): string {
    let line = "";
    for (let col = 0; col < this.#cols; col++) {
      const cell = this.cell(col, row);
      if (cell.wide === "spacer-tail" || cell.wide === "spacer-head") continue;
      line += cell.text === "" ? " " : cell.text;
    }
    return line.replace(/ +$/, "");
  }

  /** The text of every row of the active area. */
  screenText(): string[] {
    return Array.from({ length: this.#rows }, (_, row) => this.rowText(row));
  }

  /** The cursor's position in the active area and whether it is shown. */
  cursor(): Cursor {
    this.#live();
    return {
      x: this.#getU16(TERMINAL_DATA_CURSOR_X),
      y: this.#getU16(TERMINAL_DATA_CURSOR_Y),
      visible: this.#getBool(TERMINAL_DATA_CURSOR_VISIBLE),
      pendingWrap: this.#getBool(TERMINAL_DATA_CURSOR_PENDING_WRAP),
    };
  }

  /** Whether the alternate screen (used by full-screen programs) is active. */
  alternateScreen(): boolean {
    this.#live();
    this.#get(TERMINAL_DATA_ACTIVE_SCREEN);
    return this.#view().getInt32(this.#out, true) === TERMINAL_SCREEN_ALTERNATE;
  }

  /** Release the terminal and its scratch memory. The object must not be used afterwards. */
  free(): void {
    if (this.#handle === 0) return;
    this.#x.ghostty_terminal_free(this.#handle);
    this.#handle = 0;
    this.#x.ghostty_wasm_free(this.#scratch, this.#scratchLen);
    if (this.#input !== 0) this.#x.ghostty_wasm_free(this.#input, this.#inputCap);
    this.#input = 0;
    this.#inputCap = 0;
  }

  /** Reads the codepoints of the cell `#gridRef` points at. */
  #graphemeText(): string {
    const x = this.#x;
    let result = x.ghostty_grid_ref_graphemes(
      this.#gridRef,
      this.#graphemes,
      GRAPHEME_BUF_LEN,
      this.#written,
    );
    let buf = this.#graphemes;
    let big = 0;
    let bigLen = 0;
    if (result === GHOSTTY_OUT_OF_SPACE) {
      // `written` holds the required length. Clusters this long are rare.
      bigLen = this.#view().getUint32(this.#written, true);
      big = x.ghostty_wasm_alloc(bigLen * 4);
      if (big === 0) throw new VtError(-1, "ghostty_wasm_alloc");
      buf = big;
      result = x.ghostty_grid_ref_graphemes(this.#gridRef, buf, bigLen, this.#written);
    }
    try {
      this.#check(result, "grid_ref_graphemes");
      const view = this.#view();
      const count = view.getUint32(this.#written, true);
      let text = "";
      for (let i = 0; i < count; i++) {
        const codepoint = view.getUint32(buf + i * 4, true);
        if (codepoint !== 0) text += String.fromCodePoint(codepoint);
      }
      return text;
    } finally {
      if (big !== 0) x.ghostty_wasm_free(big, bigLen * 4);
    }
  }

  #get(key: number): void {
    this.#check(this.#x.ghostty_terminal_get(this.#handle, key, this.#out), "terminal_get");
  }

  #getU16(key: number): number {
    this.#get(key);
    return this.#view().getUint16(this.#out, true);
  }

  #getBool(key: number): boolean {
    this.#get(key);
    return this.#view().getUint8(this.#out) !== 0;
  }

  /** A view of the module's memory. Never keep one across a call into the module. */
  #view(): DataView {
    return new DataView(this.#x.memory.buffer);
  }

  #check(result: number, what: string): void {
    if (result !== GHOSTTY_SUCCESS) throw new VtError(result, what);
  }

  #live(): void {
    if (this.#handle === 0) throw new Error("this terminal has been freed");
  }
}

function field(layout: StructLayout, name: string): FieldLayout {
  const found = layout.fields[name];
  if (!found) throw new Error(`libghostty-vt struct has no field ${name}`);
  return found;
}
