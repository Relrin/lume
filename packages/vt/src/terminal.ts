/**
 * A terminal emulator: screen, scrollback, cursor, styles and modes, and the effects a
 * program can ask of the terminal that holds it.
 */

import { cellReader, type Cell } from "./cell.ts";
import { NO_VALUE, UNSUPPORTED } from "./error.ts";
import type { Core } from "./module.ts";

/** An opaque colour. Components are 0 to 255. */
export interface Rgb {
  r: number;
  g: number;
  b: number;
}

/** The colours a terminal starts from and reports when a program asks. */
export interface TerminalColors {
  foreground?: Rgb;
  background?: Rgb;
  cursor?: Rgb;
  /** All 256 palette entries. */
  palette?: readonly Rgb[];
}

/** Whether the surrounding interface is light or dark, for programs that ask. */
export type ColorScheme = "light" | "dark";

/** Which clipboard a program addressed. */
export type ClipboardLocation = "standard" | "selection" | "primary";

/**
 * Why the terminal produced bytes for the pty.
 *
 * - `reply`: an answer to a query in the program's output (device attributes, cursor
 *   position, a colour). Exactly one terminal may answer for a session, so the caller
 *   sends these only when it is that terminal.
 * - `input`: bytes that stand for something the user did. Always sent.
 */
export type OutputKind = "reply" | "input";

/** Receivers for what a terminal reports. Each is optional. */
export interface TerminalHandlers {
  /** Bytes to write to the pty. The array is the receiver's to keep. */
  output?(bytes: Uint8Array, kind: OutputKind): void;
  bell?(): void;
  /** The program set the window title. Not sanitised or shortened. */
  title?(title: string): void;
  /** The program reported its working directory, as it sent it. */
  pwd?(pwd: string): void;
  /** The program asked to put text on a clipboard, and the request passed the size cap. */
  clipboardWrite?(text: string, location: ClipboardLocation): void;
}

/**
 * Settings applied when a terminal is created. Each is optional, and one that is left out
 * keeps the library's default.
 */
export interface TerminalOptions {
  /**
   * Colours, applied before any output is processed so that colour queries are answered
   * from them from the first byte.
   */
  colors?: TerminalColors;
  colorScheme?: ColorScheme;
  /** The size of one cell in pixels, reported to programs that ask for pixel sizes. */
  cellWidthPx?: number;
  cellHeightPx?: number;
  /**
   * Scrollback limits. Set both: the byte limit defaults to about one page and applies
   * even when a line limit is given.
   */
  scrollbackMaxLines?: number;
  scrollbackMaxBytes?: number;
  /**
   * Whether growing the grid pulls rows back out of scrollback. Must be false for a pty
   * that redraws the screen itself after a resize, as the Windows pseudoconsole does.
   */
  resizePullsScrollback?: boolean;
  /**
   * The largest clipboard write a program may make, in bytes of text. Without it every
   * clipboard write is refused.
   */
  clipboardWriteMaxBytes?: number;
  handlers?: TerminalHandlers;
}

/** The cursor's position in the active area and whether it is shown. */
export interface Cursor {
  x: number;
  y: number;
  visible: boolean;
  pendingWrap: boolean;
}

/** Where the viewport sits in the scrollback, in rows. */
export interface Scrollbar {
  /** Rows in scrollback and screen together. */
  total: number;
  /** Row at the top of the viewport. */
  offset: number;
  /** Rows the viewport shows. */
  length: number;
}

/**
 * Numbers of the DEC private modes an interface needs to ask about. These are protocol
 * numbers, the same ones a program sends in `CSI ? n h`.
 */
export const Mode = {
  CursorKeys: 1,
  MouseX10: 9,
  CursorVisible: 25,
  MouseNormal: 1000,
  MouseButton: 1002,
  MouseAny: 1003,
  FocusEvent: 1004,
  MouseAlternateScroll: 1007,
  AlternateScreen: 1049,
  BracketedPaste: 2004,
  SynchronizedOutput: 2026,
} as const;

type Effect =
  | { type: "output"; bytes: Uint8Array; kind: OutputKind }
  | { type: "bell" }
  | { type: "title"; text: string }
  | { type: "pwd"; text: string }
  | { type: "clipboard"; text: string; location: ClipboardLocation };

/** What the shared callbacks call on the terminal a callback names. */
interface Sink {
  writePty(data: number, len: number): void;
  bell(): void;
  titleChanged(): void;
  pwdChanged(): void;
  size(out: number): number;
  colorScheme(out: number): number;
  clipboardWrite(request: number): void;
}

/** The callbacks of one module instance, shared by all of its terminals. */
interface Shared {
  readonly sinks: Map<number, Sink>;
  readonly options: readonly (readonly [option: string, index: number])[];
}

const sharedByCore = new WeakMap<Core, Shared>();

/**
 * Install the terminal callbacks once per module.
 *
 * Every callback receives the handle of the terminal it concerns, so one set serves all
 * terminals and the function table grows by a fixed amount however many terminals exist.
 */
function shared(core: Core): Shared {
  const existing = sharedByCore.get(core);
  if (existing) return existing;

  const { manifest, trampoline } = core;
  const sinks = new Map<number, Sink>();
  const install = (type: string, call: (sink: Sink, a: number, b: number) => number | void) =>
    trampoline.install(manifest.callback(type), (terminal: number, _userdata: number, ...args) => {
      const sink = sinks.get(terminal);
      // A boolean callback that returns false declines to answer.
      return sink ? (call(sink, args[0] as number, args[1] as number) ?? 0) : 0;
    });

  const created: Shared = {
    sinks,
    options: [
      ["WRITE_PTY", install("GhosttyTerminalWritePtyFn", (s, data, len) => s.writePty(data, len))],
      ["BELL", install("GhosttyTerminalBellFn", (s) => s.bell())],
      ["TITLE_CHANGED", install("GhosttyTerminalTitleChangedFn", (s) => s.titleChanged())],
      ["PWD_CHANGED", install("GhosttyTerminalPwdChangedFn", (s) => s.pwdChanged())],
      ["SIZE", install("GhosttyTerminalSizeFn", (s, out) => s.size(out))],
      ["COLOR_SCHEME", install("GhosttyTerminalColorSchemeFn", (s, out) => s.colorScheme(out))],
      [
        "CLIPBOARD_WRITE",
        install("GhosttyTerminalClipboardWriteFn", (s, request) => s.clipboardWrite(request)),
      ],
    ],
  };
  sharedByCore.set(core, created);
  return created;
}

/** Bytes of scratch memory for values read from the terminal. */
const OUT_BYTES = 32;
/** Bytes of scratch memory for values passed to the terminal. */
const ARG_BYTES = 32;
const PALETTE_ENTRIES = 256;

/** A terminal emulator: screen, scrollback, cursor, styles and modes. */
export class Terminal {
  /** @internal The module this terminal lives in. */
  readonly core: Core;
  /** Receivers for what the terminal reports. May be replaced at any time. */
  handlers: TerminalHandlers;

  #handle: number;
  #cols: number;
  #rows: number;
  #cellWidthPx: number;
  #cellHeightPx: number;
  #colorScheme: ColorScheme | undefined;
  #clipboardWriteMaxBytes: number | undefined;
  #scrollbackMaxBytes: number | undefined;
  #revision = 0;

  readonly #scratch: number;
  readonly #out: number;
  readonly #arg: number;

  // Effects are collected while the module runs and delivered once it has returned.
  #effects: Effect[] = [];
  #origin: OutputKind = "input";
  #depth = 0;
  #flushing = false;
  #callbackError: unknown;

  readonly #option: Readonly<Record<string, number>>;
  readonly #data: Readonly<Record<string, number>>;

  /** @internal Use `Vt.createTerminal`. */
  constructor(core: Core, cols: number, rows: number, options?: TerminalOptions) {
    this.core = core;
    this.#cols = cols;
    this.#rows = rows;
    this.#cellWidthPx = options?.cellWidthPx ?? 0;
    this.#cellHeightPx = options?.cellHeightPx ?? 0;
    this.#colorScheme = options?.colorScheme;
    this.#clipboardWriteMaxBytes = options?.clipboardWriteMaxBytes;
    this.#scrollbackMaxBytes = options?.scrollbackMaxBytes;
    this.handlers = options?.handlers ?? {};
    this.#option = core.manifest.enumValues("GhosttyTerminalOption");
    this.#data = core.manifest.enumValues("GhosttyTerminalData");

    const callbacks = shared(core);
    this.#scratch = core.allocZeroed(OUT_BYTES + ARG_BYTES);
    this.#out = this.#scratch;
    this.#arg = this.#scratch + OUT_BYTES;
    try {
      this.#handle = core.newHandle("ghostty_terminal_new", (out) =>
        core.x.ghostty_terminal_new(0, out, cols, rows),
      );
    } catch (err) {
      core.free(this.#scratch, OUT_BYTES + ARG_BYTES);
      throw err;
    }

    try {
      callbacks.sinks.set(this.#handle, this.#sink());
      for (const [option, index] of callbacks.options) this.#set(option, index);
      if (options) this.#apply(options);
    } catch (err) {
      this.free();
      throw err;
    }
  }

  /** @internal The library's handle, or 0 once freed. */
  get handle(): number {
    return this.#handle;
  }

  /**
   * A number that changes whenever output, a resize or a reset may have changed the
   * terminal's modes. Encoders use it to know when to read the modes again.
   */
  get revision(): number {
    return this.#revision;
  }

  get cols(): number {
    return this.#cols;
  }

  get rows(): number {
    return this.#rows;
  }

  /**
   * Feed terminal output (text and escape sequences) into the terminal.
   *
   * Anything the output makes the terminal report is delivered to `handlers` before this
   * returns, after the terminal has finished with the bytes. A handler may therefore
   * call back into the terminal.
   */
  write(data: Uint8Array | string): void {
    this.#live();
    if (data.length === 0) return;
    this.#call("reply", () => {
      const { core } = this;
      if (typeof data === "string") {
        const { ptr, len } = core.stageText(data);
        core.x.ghostty_terminal_vt_write(this.#handle, ptr, len);
      } else {
        core.x.ghostty_terminal_vt_write(this.#handle, core.stage(data), data.length);
      }
      this.#revision++;
    });
  }

  /**
   * Resize the grid. The cell size in pixels is kept from the last call that gave one;
   * programs that ask for pixel sizes are answered from it.
   */
  resize(cols: number, rows: number, cellWidthPx?: number, cellHeightPx?: number): void {
    this.#live();
    const width = cellWidthPx ?? this.#cellWidthPx;
    const height = cellHeightPx ?? this.#cellHeightPx;
    // A resize can produce an in-band size report, which answers the program's request
    // for such reports and so counts as a reply.
    this.#call("reply", () => {
      this.core.check(
        this.core.x.ghostty_terminal_resize(this.#handle, cols, rows, width, height),
        "ghostty_terminal_resize",
      );
      this.#cols = cols;
      this.#rows = rows;
      this.#cellWidthPx = width;
      this.#cellHeightPx = height;
      this.#revision++;
    });
  }

  /** Return the terminal to its initial state, as a full reset sequence would. */
  reset(): void {
    this.#live();
    this.#call("input", () => {
      this.core.x.ghostty_terminal_reset(this.#handle);
      this.#revision++;
    });
  }

  /** Change colours. A colour that is left out is not changed. */
  setColors(colors: TerminalColors): void {
    this.#live();
    this.#call("input", () => this.#setColors(colors));
  }

  /** Tell programs that ask whether the interface is light or dark. */
  setColorScheme(scheme: ColorScheme): void {
    this.#colorScheme = scheme;
  }

  /** The cell at `col`, `row` of the active area (the screen without scrollback). */
  cell(col: number, row: number): Cell {
    this.#live();
    if (col < 0 || row < 0 || col >= this.#cols || row >= this.#rows) {
      throw new RangeError(`cell (${col}, ${row}) is outside ${this.#cols}x${this.#rows}`);
    }
    return cellReader(this.core).read(this.#handle, col, row);
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
      x: this.#getU16("CURSOR_X"),
      y: this.#getU16("CURSOR_Y"),
      visible: this.#getBool("CURSOR_VISIBLE"),
      pendingWrap: this.#getBool("CURSOR_PENDING_WRAP"),
    };
  }

  /** Whether the alternate screen (used by full-screen programs) is active. */
  alternateScreen(): boolean {
    this.#live();
    this.#get("ACTIVE_SCREEN");
    const alternate = this.core.manifest.enumValue("GhosttyTerminalScreen", "ALTERNATE");
    return this.core.view().getInt32(this.#out, true) === alternate;
  }

  /** The window title the program last set, or `""`. */
  title(): string {
    this.#live();
    return this.#getString("TITLE");
  }

  /** The working directory the program last reported, or `""`. */
  pwd(): string {
    this.#live();
    return this.#getString("PWD");
  }

  /**
   * Whether a mode is set. `mode` is the number a program uses for it; `ansi` selects
   * the ANSI modes (`CSI n h`) instead of the DEC private ones (`CSI ? n h`).
   */
  mode(mode: number, ansi = false): boolean {
    this.#live();
    const { core } = this;
    const m = core.manifest;
    const view = core.view();
    view.setUint32(this.#out, 0, true);
    // The library packs the mode number and its kind into one 16-bit value.
    const packed = (mode & 0x7fff) | (ansi ? 0x8000 : 0);
    view.setUint16(this.#out + m.offset("GhosttyTerminalModeConfig", "mode"), packed, true);
    this.#get("MODE");
    return core.view().getUint8(this.#out + m.offset("GhosttyTerminalModeConfig", "value")) !== 0;
  }

  /** Whether the program has asked for mouse events in any form. */
  mouseTracking(): boolean {
    this.#live();
    return this.#getBool("MOUSE_TRACKING");
  }

  /** Whether the viewport shows the live screen rather than scrollback. */
  viewportAtBottom(): boolean {
    this.#live();
    return this.#getBool("VIEWPORT_ACTIVE");
  }

  /** Where the viewport sits in the scrollback. It is not announced: ask when drawing. */
  scrollbar(): Scrollbar {
    this.#live();
    this.#get("SCROLLBAR");
    const view = this.core.view();
    const at = (field: string) =>
      Number(
        view.getBigUint64(
          this.#out + this.core.manifest.offset("GhosttyTerminalScrollbar", field),
          true,
        ),
      );
    return { total: at("total"), offset: at("offset"), length: at("len") };
  }

  /** Show the oldest scrollback. */
  scrollToTop(): void {
    this.#scroll("TOP", 0);
  }

  /** Show the live screen. */
  scrollToBottom(): void {
    this.#scroll("BOTTOM", 0);
  }

  /** Move the viewport by `rows`; negative is towards older output. */
  scrollBy(rows: number): void {
    this.#scroll("DELTA", rows);
  }

  /** Put `row`, counted as `Scrollbar.offset` is, at the top of the viewport. */
  scrollToRow(row: number): void {
    this.#scroll("ROW", row);
  }

  /**
   * Forget everything that has scrolled off the screen. The screen itself is kept.
   *
   * This is done through the scrollback limit, not by feeding the terminal an escape
   * sequence: a sequence written between two chunks of output could land in the middle
   * of one the program is sending.
   */
  clearScrollback(): void {
    this.#live();
    this.#call("input", () => {
      // A byte limit of zero erases the history. The limit in force is then put back;
      // the address 0 restores the library's default.
      this.#setSize("SCROLLBACK_MAX_BYTES", 0);
      if (this.#scrollbackMaxBytes === undefined) this.#set("SCROLLBACK_MAX_BYTES", 0);
      else this.#setSize("SCROLLBACK_MAX_BYTES", this.#scrollbackMaxBytes);
      this.#revision++;
    });
  }

  /** Release the terminal and its scratch memory. The object must not be used afterwards. */
  free(): void {
    if (this.#handle === 0) return;
    sharedByCore.get(this.core)?.sinks.delete(this.#handle);
    this.core.x.ghostty_terminal_free(this.#handle);
    this.#handle = 0;
    this.core.free(this.#scratch, OUT_BYTES + ARG_BYTES);
    this.#effects = [];
  }

  /**
   * @internal Run a call into the module that may make the terminal report something.
   * `origin` says what bytes for the pty mean if the call produces any.
   */
  run<T>(origin: OutputKind, call: () => T): T {
    this.#live();
    return this.#call(origin, call);
  }

  /**
   * @internal Set an option by its name in the library's option list. `value` is the
   * address of the value, or 0 to restore the default.
   */
  setOption(option: string, value: number): void {
    this.#live();
    this.#call("input", () => this.#set(option, value));
  }

  /**
   * @internal Read a value by its name in the library's data list into memory at `out`.
   * Returns false when the terminal has no such value at the moment.
   */
  getData(key: string, out: number): boolean {
    this.#live();
    const value = this.#data[key];
    if (value === undefined) throw new Error(`libghostty-vt has no terminal data ${key}`);
    const result = this.core.x.ghostty_terminal_get(this.#handle, value, out);
    if (result === NO_VALUE) return false;
    this.core.check(result, `ghostty_terminal_get(${key})`);
    return true;
  }

  #call<T>(origin: OutputKind, call: () => T): T {
    const outer = this.#origin;
    this.#origin = origin;
    this.#depth++;
    try {
      return call();
    } finally {
      this.#origin = outer;
      this.#depth--;
      if (this.#depth === 0) this.#settle();
    }
  }

  /** Deliver what the module reported, now that it is no longer running. */
  #settle(): void {
    const error = this.#callbackError;
    this.#callbackError = undefined;
    if (!this.#flushing) {
      this.#flushing = true;
      try {
        // A handler may call back in and add effects; they are delivered in order here.
        for (let i = 0; i < this.#effects.length; i++) this.#deliver(this.#effects[i]!);
      } finally {
        this.#effects = [];
        this.#flushing = false;
      }
    }
    if (error !== undefined) throw error;
  }

  #deliver(effect: Effect): void {
    const handlers = this.handlers;
    switch (effect.type) {
      case "output":
        handlers.output?.(effect.bytes, effect.kind);
        break;
      case "bell":
        handlers.bell?.();
        break;
      case "title":
        handlers.title?.(effect.text);
        break;
      case "pwd":
        handlers.pwd?.(effect.text);
        break;
      case "clipboard":
        handlers.clipboardWrite?.(effect.text, effect.location);
        break;
    }
  }

  /**
   * The receivers the module's callbacks reach. They run while the module is in the
   * middle of a call: the library is not re-entrant, and an exception must not unwind
   * through it. So each one only records what happened.
   */
  #sink(): Sink {
    const guard = <T>(fallback: T, body: () => T): T => {
      try {
        return body();
      } catch (err) {
        this.#callbackError ??= err;
        return fallback;
      }
    };
    return {
      writePty: (data, len) =>
        guard(undefined, () => {
          // The bytes are valid only during the callback.
          this.#effects.push({
            type: "output",
            bytes: this.core.copy(data, len),
            kind: this.#origin,
          });
        }),
      bell: () => void this.#effects.push({ type: "bell" }),
      // The library says to read the new value from inside the callback.
      titleChanged: () =>
        guard(undefined, () => {
          this.#effects.push({ type: "title", text: this.#getString("TITLE") });
        }),
      pwdChanged: () =>
        guard(undefined, () => {
          this.#effects.push({ type: "pwd", text: this.#getString("PWD") });
        }),
      size: (out) =>
        guard(0, () => {
          const m = this.core.manifest;
          const view = this.core.view();
          view.setUint16(out + m.offset("GhosttySizeReportSize", "rows"), this.#rows, true);
          view.setUint16(out + m.offset("GhosttySizeReportSize", "columns"), this.#cols, true);
          view.setUint32(
            out + m.offset("GhosttySizeReportSize", "cell_width"),
            this.#cellWidthPx,
            true,
          );
          view.setUint32(
            out + m.offset("GhosttySizeReportSize", "cell_height"),
            this.#cellHeightPx,
            true,
          );
          return 1;
        }),
      colorScheme: (out) =>
        guard(0, () => {
          if (this.#colorScheme === undefined) return 0;
          const name = this.#colorScheme === "dark" ? "DARK" : "LIGHT";
          const value = this.core.manifest.enumValue("GhosttyColorScheme", name);
          this.core.view().setInt32(out, value, true);
          return 1;
        }),
      clipboardWrite: (request) => guard(undefined, () => this.#clipboardWrite(request)),
    };
  }

  /**
   * Answer a program's request to write the clipboard.
   *
   * Only plain text within the configured cap is accepted, and the text is handed on as
   * an effect: whether it reaches a clipboard is the receiver's decision. Returning
   * without replying is how the library is told the write is refused.
   */
  #clipboardWrite(request: number): void {
    const { core } = this;
    const m = core.manifest;
    const cap = this.#clipboardWriteMaxBytes;
    if (cap === undefined) return;

    const view = core.view();
    const contents = view.getUint32(request + m.offset("GhosttyClipboardWrite", "contents"), true);
    const count = view.getUint32(request + m.offset("GhosttyClipboardWrite", "contents_len"), true);
    const stringPtr = m.offset("GhosttyString", "ptr");
    const stringLen = m.offset("GhosttyString", "len");
    const content = m.struct("GhosttyClipboardContent");
    const mimeAt = m.offset("GhosttyClipboardContent", "mime");
    const dataAt = m.offset("GhosttyClipboardContent", "data");

    // No representation at all asks for the clipboard to be cleared.
    let text: string | undefined = count === 0 ? "" : undefined;
    for (let i = 0; i < count && text === undefined; i++) {
      const entry = contents + i * content.size;
      const mime = core.string(
        view.getUint32(entry + mimeAt + stringPtr, true),
        view.getUint32(entry + mimeAt + stringLen, true),
      );
      if (!/^text\/plain(;|$)/.test(mime)) continue;
      const len = view.getUint32(entry + dataAt + stringLen, true);
      if (len > cap) return;
      text = core.string(view.getUint32(entry + dataAt + stringPtr, true), len);
    }
    if (text === undefined) return;

    const locations = m.enumValues("GhosttyClipboardLocation");
    const raw = view.getInt32(request + m.offset("GhosttyClipboardWrite", "location"), true);
    const location: ClipboardLocation =
      raw === locations.SELECTION
        ? "selection"
        : raw === locations.PRIMARY
          ? "primary"
          : "standard";
    this.#effects.push({ type: "clipboard", text, location });

    // Protocols that acknowledge a write are told it succeeded. The reply is a sized
    // struct and is borrowed only for the duration of the reply call.
    const reply = m.struct("GhosttyClipboardWriteReply");
    core.x.ghostty_sized_init(this.#arg, reply.size);
    core
      .view()
      .setInt32(
        this.#arg + m.offset("GhosttyClipboardWriteReply", "result"),
        m.enumValue("GhosttyClipboardWriteResult", "SUCCESS"),
        true,
      );
    const replyFn = core
      .view()
      .getUint32(request + m.offset("GhosttyClipboardWrite", "reply"), true);
    core.trampoline.get(replyFn)(request, this.#arg);
  }

  #apply(options: TerminalOptions): void {
    if (options.colors) this.#setColors(options.colors);
    if (options.scrollbackMaxBytes !== undefined) {
      this.#setSize("SCROLLBACK_MAX_BYTES", options.scrollbackMaxBytes);
    }
    if (options.scrollbackMaxLines !== undefined) {
      this.#setSize("SCROLLBACK_MAX_LINES", options.scrollbackMaxLines);
    }
    if (options.resizePullsScrollback !== undefined) {
      this.#setBool("RESIZE_PULL_SCROLLBACK", options.resizePullsScrollback);
    }

    // Inline images are not drawn, so none are stored. A build without image support
    // does not know the option, which amounts to the same thing.
    const view = this.core.view();
    view.setBigUint64(this.#arg, 0n, true);
    const result = this.core.x.ghostty_terminal_set(
      this.#handle,
      this.#optionValue("KITTY_IMAGE_STORAGE_LIMIT"),
      this.#arg,
    );
    if (result !== UNSUPPORTED) this.core.check(result, "ghostty_terminal_set");

    // Reporting the title back lets output inject text into the program's input.
    this.#setBool("TITLE_REPORT", false);
  }

  #setColors(colors: TerminalColors): void {
    const rgb = (option: string, color: Rgb | undefined) => {
      if (!color) return;
      this.core.u8().set([color.r, color.g, color.b], this.#arg);
      this.#set(option, this.#arg);
    };
    rgb("COLOR_FOREGROUND", colors.foreground);
    rgb("COLOR_BACKGROUND", colors.background);
    rgb("COLOR_CURSOR", colors.cursor);

    const palette = colors.palette;
    if (!palette) return;
    if (palette.length !== PALETTE_ENTRIES) {
      throw new RangeError(`a palette has ${PALETTE_ENTRIES} entries, not ${palette.length}`);
    }
    const bytes = new Uint8Array(PALETTE_ENTRIES * 3);
    palette.forEach((color, i) => bytes.set([color.r, color.g, color.b], i * 3));
    this.#set("COLOR_PALETTE", this.core.stage(bytes));
  }

  #scroll(tag: string, amount: number): void {
    this.#live();
    const m = this.core.manifest;
    const layout = m.struct("GhosttyTerminalScrollViewport");
    this.core.u8().fill(0, this.#arg, this.#arg + layout.size);
    const view = this.core.view();
    view.setInt32(
      this.#arg + m.offset("GhosttyTerminalScrollViewport", "tag"),
      m.enumValue("GhosttyTerminalScrollViewportTag", tag),
      true,
    );
    // Every arm of the value is one pointer-sized integer at the same place.
    view.setInt32(this.#arg + m.offset("GhosttyTerminalScrollViewport", "value"), amount, true);
    // The struct is passed by value in C; the export takes its address.
    this.core.check(
      this.core.x.ghostty_terminal_scroll_viewport(this.#handle, this.#arg),
      "ghostty_terminal_scroll_viewport",
    );
  }

  #optionValue(option: string): number {
    const value = this.#option[option];
    if (value === undefined) throw new Error(`libghostty-vt has no terminal option ${option}`);
    return value;
  }

  /** Set an option. `value` is the value itself for callbacks, otherwise its address. */
  #set(option: string, value: number): void {
    this.core.check(
      this.core.x.ghostty_terminal_set(this.#handle, this.#optionValue(option), value),
      `ghostty_terminal_set(${option})`,
    );
  }

  #setBool(option: string, value: boolean): void {
    this.core.view().setUint8(this.#arg, value ? 1 : 0);
    this.#set(option, this.#arg);
  }

  #setSize(option: string, value: number): void {
    this.core.view().setUint32(this.#arg, value, true);
    this.#set(option, this.#arg);
  }

  /** Read a value into `#out`. Returns false when the terminal has none. */
  #get(key: string): boolean {
    return this.getData(key, this.#out);
  }

  #getU16(key: string): number {
    this.#get(key);
    return this.core.view().getUint16(this.#out, true);
  }

  #getBool(key: string): boolean {
    this.#get(key);
    return this.core.view().getUint8(this.#out) !== 0;
  }

  #getString(key: string): string {
    if (!this.#get(key)) return "";
    const m = this.core.manifest;
    const view = this.core.view();
    return this.core.string(
      view.getUint32(this.#out + m.offset("GhosttyString", "ptr"), true),
      view.getUint32(this.#out + m.offset("GhosttyString", "len"), true),
    );
  }

  #live(): void {
    if (this.#handle === 0) throw new Error("this terminal has been freed");
  }
}
