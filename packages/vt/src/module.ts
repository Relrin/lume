/**
 * The instantiated libghostty-vt module and the helpers every wrapper class shares:
 * typed exports, views of the module's memory, allocation and handle creation.
 */

import { OUT_OF_MEMORY, OUT_OF_SPACE, SUCCESS, VtError } from "./error.ts";
import { Manifest } from "./manifest.ts";
import { Terminal, type TerminalOptions } from "./terminal.ts";
import { Trampoline } from "./trampoline.ts";

/**
 * The exports this package calls. Pointers, handles, sizes, enums and booleans are
 * numbers (wasm32); `GhosttyCell` is a 64-bit integer.
 *
 * Where a C function takes a struct by value, the export takes the address of the struct
 * instead. Where a C function returns a struct, the export takes the address to write the
 * result to as an extra first argument.
 */
export interface Exports {
  memory: WebAssembly.Memory;
  __indirect_function_table: WebAssembly.Table;

  ghostty_wasm_alloc(len: number): number;
  ghostty_wasm_free(ptr: number, len: number): void;
  ghostty_wasm_alloc_opaque(): number;
  ghostty_wasm_take_opaque(slot: number): number;
  ghostty_wasm_free_opaque(slot: number): void;
  ghostty_version(): number;
  ghostty_type_json(): number;
  ghostty_sized_init(ptr: number, size: number): void;
  ghostty_free(allocator: number, ptr: number, len: number): void;
  ghostty_sys_set(option: number, value: number): number;

  ghostty_terminal_new(allocator: number, out: number, cols: number, rows: number): number;
  ghostty_terminal_free(terminal: number): void;
  ghostty_terminal_reset(terminal: number): void;
  ghostty_terminal_vt_write(terminal: number, data: number, len: number): void;
  ghostty_terminal_resize(
    terminal: number,
    cols: number,
    rows: number,
    cellWidthPx: number,
    cellHeightPx: number,
  ): number;
  ghostty_terminal_set(terminal: number, option: number, value: number): number;
  ghostty_terminal_get(terminal: number, key: number, out: number): number;
  ghostty_terminal_scroll_viewport(terminal: number, behavior: number): number;
  ghostty_terminal_grid_ref(terminal: number, point: number, out: number): number;
  ghostty_terminal_point_from_grid_ref(
    terminal: number,
    ref: number,
    tag: number,
    out: number,
  ): number;
  ghostty_terminal_grid_ref_track(terminal: number, point: number, out: number): number;
  ghostty_tracked_grid_ref_free(ref: number): void;
  ghostty_tracked_grid_ref_point(ref: number, tag: number, out: number): number;
  ghostty_tracked_grid_ref_set(ref: number, terminal: number, point: number): number;
  ghostty_tracked_grid_ref_snapshot(ref: number, out: number): number;
  ghostty_terminal_select_all(terminal: number, out: number): number;
  ghostty_terminal_select_word(terminal: number, options: number, out: number): number;
  ghostty_terminal_select_line(terminal: number, options: number, out: number): number;

  ghostty_grid_ref_cell(ref: number, out: number): number;
  ghostty_grid_ref_style(ref: number, out: number): number;
  ghostty_grid_ref_graphemes(ref: number, buf: number, bufLen: number, outWritten: number): number;
  ghostty_grid_ref_hyperlink_uri(
    ref: number,
    buf: number,
    bufLen: number,
    outWritten: number,
  ): number;
  ghostty_cell_get(cell: bigint, key: number, out: number): number;

  ghostty_render_state_new(allocator: number, out: number): number;
  ghostty_render_state_free(state: number): void;
  ghostty_render_state_update(state: number, terminal: number): number;
  ghostty_render_state_clean(state: number): number;
  ghostty_render_state_get(state: number, key: number, out: number): number;
  ghostty_render_state_row_iterator_new(allocator: number, out: number): number;
  ghostty_render_state_row_iterator_free(iterator: number): void;
  ghostty_render_state_row_iterator_next(iterator: number): number;
  ghostty_render_state_row_iterator_next_dirty(iterator: number, outY: number): number;
  ghostty_render_state_row_get(iterator: number, key: number, out: number): number;
  ghostty_render_state_row_cells_new(allocator: number, out: number): number;
  ghostty_render_state_row_cells_free(cells: number): void;
  ghostty_render_state_row_cells_select(cells: number, x: number): number;
  ghostty_render_state_row_cells_get(cells: number, key: number, out: number): number;

  ghostty_key_encoder_new(allocator: number, out: number): number;
  ghostty_key_encoder_free(encoder: number): void;
  ghostty_key_encoder_set_from_terminal(encoder: number, terminal: number): number;
  ghostty_key_encoder_encode(
    encoder: number,
    event: number,
    buf: number,
    bufLen: number,
    outWritten: number,
  ): number;
  ghostty_key_event_new(allocator: number, out: number): number;
  ghostty_key_event_free(event: number): void;
  ghostty_key_event_set_action(event: number, action: number): number;
  ghostty_key_event_set_key(event: number, key: number): number;
  ghostty_key_event_set_mods(event: number, mods: number): number;
  ghostty_key_event_set_consumed_mods(event: number, mods: number): number;
  ghostty_key_event_set_composing(event: number, composing: number): number;
  ghostty_key_event_set_utf8(event: number, utf8: number): number;
  ghostty_key_event_set_unshifted_codepoint(event: number, codepoint: number): number;

  ghostty_mouse_encoder_new(allocator: number, out: number): number;
  ghostty_mouse_encoder_free(encoder: number): void;
  ghostty_mouse_encoder_set(encoder: number, option: number, value: number): number;
  ghostty_mouse_encoder_set_from_terminal(encoder: number, terminal: number): number;
  ghostty_mouse_encoder_reset(encoder: number): number;
  ghostty_mouse_encoder_encode(
    encoder: number,
    event: number,
    buf: number,
    bufLen: number,
    outWritten: number,
  ): number;
  ghostty_mouse_event_new(allocator: number, out: number): number;
  ghostty_mouse_event_free(event: number): void;
  ghostty_mouse_event_set_action(event: number, action: number): number;
  ghostty_mouse_event_set_button(event: number, button: number): number;
  ghostty_mouse_event_clear_button(event: number): number;
  ghostty_mouse_event_set_mods(event: number, mods: number): number;
  ghostty_mouse_event_set_position(event: number, position: number): number;

  ghostty_focus_encode(event: number, buf: number, bufLen: number, outWritten: number): number;
  ghostty_color_scheme_report_encode(
    scheme: number,
    buf: number,
    bufLen: number,
    outWritten: number,
  ): number;
  ghostty_paste_is_safe(data: number, len: number): number;
  ghostty_paste_encode(
    data: number,
    dataLen: number,
    bracketed: number,
    buf: number,
    bufLen: number,
    outWritten: number,
  ): number;

  ghostty_selection_gesture_new(allocator: number, out: number): number;
  ghostty_selection_gesture_free(gesture: number, terminal: number): void;
  ghostty_selection_gesture_reset(gesture: number, terminal: number): void;
  ghostty_selection_gesture_get(
    gesture: number,
    terminal: number,
    key: number,
    out: number,
  ): number;
  ghostty_selection_gesture_event(
    gesture: number,
    terminal: number,
    event: number,
    outSelection: number,
  ): number;
  ghostty_selection_gesture_event_new(allocator: number, out: number, type: number): number;
  ghostty_selection_gesture_event_free(event: number): void;
  ghostty_selection_gesture_event_set(event: number, option: number, value: number): number;

  ghostty_formatter_terminal_new(allocator: number, out: number, terminal: number): number;
  ghostty_formatter_free(formatter: number): void;
  ghostty_formatter_set(formatter: number, option: number, value: number): number;
  ghostty_formatter_format_alloc(
    formatter: number,
    allocator: number,
    outPtr: number,
    outLen: number,
  ): number;

  ghostty_search_new(allocator: number, out: number, terminal: number): number;
  ghostty_search_free(search: number): void;
  ghostty_search_tick(search: number, outStatus: number): number;
  ghostty_search_feed(search: number): number;
  ghostty_search_set(search: number, option: number, value: number): number;
  ghostty_search_get(search: number, data: number, value: number): number;
}

// Written as a record so that the compiler rejects a name missing from either side.
const REQUIRED_EXPORTS: Readonly<Record<keyof Exports, true>> = {
  memory: true,
  __indirect_function_table: true,
  ghostty_wasm_alloc: true,
  ghostty_wasm_free: true,
  ghostty_wasm_alloc_opaque: true,
  ghostty_wasm_take_opaque: true,
  ghostty_wasm_free_opaque: true,
  ghostty_version: true,
  ghostty_type_json: true,
  ghostty_sized_init: true,
  ghostty_free: true,
  ghostty_sys_set: true,
  ghostty_terminal_new: true,
  ghostty_terminal_free: true,
  ghostty_terminal_reset: true,
  ghostty_terminal_vt_write: true,
  ghostty_terminal_resize: true,
  ghostty_terminal_set: true,
  ghostty_terminal_get: true,
  ghostty_terminal_scroll_viewport: true,
  ghostty_terminal_grid_ref: true,
  ghostty_terminal_point_from_grid_ref: true,
  ghostty_terminal_grid_ref_track: true,
  ghostty_tracked_grid_ref_free: true,
  ghostty_tracked_grid_ref_point: true,
  ghostty_tracked_grid_ref_set: true,
  ghostty_tracked_grid_ref_snapshot: true,
  ghostty_terminal_select_all: true,
  ghostty_terminal_select_word: true,
  ghostty_terminal_select_line: true,
  ghostty_grid_ref_cell: true,
  ghostty_grid_ref_style: true,
  ghostty_grid_ref_graphemes: true,
  ghostty_grid_ref_hyperlink_uri: true,
  ghostty_cell_get: true,
  ghostty_render_state_new: true,
  ghostty_render_state_free: true,
  ghostty_render_state_update: true,
  ghostty_render_state_clean: true,
  ghostty_render_state_get: true,
  ghostty_render_state_row_iterator_new: true,
  ghostty_render_state_row_iterator_free: true,
  ghostty_render_state_row_iterator_next: true,
  ghostty_render_state_row_iterator_next_dirty: true,
  ghostty_render_state_row_get: true,
  ghostty_render_state_row_cells_new: true,
  ghostty_render_state_row_cells_free: true,
  ghostty_render_state_row_cells_select: true,
  ghostty_render_state_row_cells_get: true,
  ghostty_key_encoder_new: true,
  ghostty_key_encoder_free: true,
  ghostty_key_encoder_set_from_terminal: true,
  ghostty_key_encoder_encode: true,
  ghostty_key_event_new: true,
  ghostty_key_event_free: true,
  ghostty_key_event_set_action: true,
  ghostty_key_event_set_key: true,
  ghostty_key_event_set_mods: true,
  ghostty_key_event_set_consumed_mods: true,
  ghostty_key_event_set_composing: true,
  ghostty_key_event_set_utf8: true,
  ghostty_key_event_set_unshifted_codepoint: true,
  ghostty_mouse_encoder_new: true,
  ghostty_mouse_encoder_free: true,
  ghostty_mouse_encoder_set: true,
  ghostty_mouse_encoder_set_from_terminal: true,
  ghostty_mouse_encoder_reset: true,
  ghostty_mouse_encoder_encode: true,
  ghostty_mouse_event_new: true,
  ghostty_mouse_event_free: true,
  ghostty_mouse_event_set_action: true,
  ghostty_mouse_event_set_button: true,
  ghostty_mouse_event_clear_button: true,
  ghostty_mouse_event_set_mods: true,
  ghostty_mouse_event_set_position: true,
  ghostty_focus_encode: true,
  ghostty_color_scheme_report_encode: true,
  ghostty_paste_is_safe: true,
  ghostty_paste_encode: true,
  ghostty_selection_gesture_new: true,
  ghostty_selection_gesture_free: true,
  ghostty_selection_gesture_reset: true,
  ghostty_selection_gesture_get: true,
  ghostty_selection_gesture_event: true,
  ghostty_selection_gesture_event_new: true,
  ghostty_selection_gesture_event_free: true,
  ghostty_selection_gesture_event_set: true,
  ghostty_formatter_terminal_new: true,
  ghostty_formatter_free: true,
  ghostty_formatter_set: true,
  ghostty_formatter_format_alloc: true,
  ghostty_search_new: true,
  ghostty_search_free: true,
  ghostty_search_tick: true,
  ghostty_search_feed: true,
  ghostty_search_set: true,
  ghostty_search_get: true,
};

/** The most bytes one `crypto.getRandomValues` call will fill. */
const RANDOM_CHUNK_BYTES = 65536;

/** The staging buffer starts at this size and grows to the largest input seen. */
const STAGING_MIN_BYTES = 64 * 1024;

/** The buffer encoders write into starts at this size and grows when one needs more. */
const ENCODE_MIN_BYTES = 256;

/** Where a log line from the library came from and how serious it is. */
export type LogLevel = "error" | "warning" | "info" | "debug";

/** Receives the library's own log lines. */
export type LogSink = (level: LogLevel, scope: string, message: string) => void;

/** Options for `Vt.instantiate`. */
export interface VtOptions {
  /** Where the library's log lines go. Without it they are dropped. */
  log?: LogSink;
}

/**
 * The module instance with the helpers the wrapper classes share. Internal to this
 * package: it exposes raw addresses.
 */
export class Core {
  readonly x: Exports;
  readonly manifest: Manifest;
  readonly trampoline: Trampoline;
  /** The library version the module reports, for example `"1.0.1"`. */
  readonly version: string;

  readonly #encoder = new TextEncoder();
  readonly #decoder = new TextDecoder();
  #bytes: Uint8Array<ArrayBuffer> | undefined;
  #view: DataView | undefined;
  #words: Uint32Array | undefined;
  /** One slot for the out-parameter of every handle constructor. */
  readonly #handleSlot: number;
  #staging = 0;
  #stagingCap = 0;
  #encoded = 0;
  #encodedCap = 0;
  /** Where encoders report how many bytes they wrote. */
  readonly #written: number;

  constructor(exports: Exports) {
    this.x = exports;
    this.version = this.cString(exports.ghostty_version());
    this.manifest = new Manifest(this.cString(exports.ghostty_type_json()));
    this.trampoline = new Trampoline(exports.__indirect_function_table);
    this.#handleSlot = exports.ghostty_wasm_alloc_opaque();
    if (this.#handleSlot === 0) throw new VtError(OUT_OF_MEMORY, "ghostty_wasm_alloc_opaque");
    this.#written = this.alloc(4);
  }

  /**
   * The module's memory as bytes.
   *
   * Any call into the module may grow its memory, which replaces the underlying buffer
   * and leaves earlier views empty. Take a view after the call whose results it reads,
   * and never keep one across a call.
   */
  u8(): Uint8Array<ArrayBuffer> {
    if (this.#bytes?.buffer !== this.x.memory.buffer) {
      this.#bytes = new Uint8Array(this.x.memory.buffer);
    }
    return this.#bytes;
  }

  /** The module's memory as a `DataView`. The same caution as `u8` applies. */
  view(): DataView {
    if (this.#view?.buffer !== this.x.memory.buffer) {
      this.#view = new DataView(this.x.memory.buffer);
    }
    return this.#view;
  }

  /**
   * The module's memory as 32-bit words, indexed by address divided by four. The same
   * caution as `u8` applies.
   */
  u32(): Uint32Array {
    if (this.#words?.buffer !== this.x.memory.buffer) {
      this.#words = new Uint32Array(this.x.memory.buffer);
    }
    return this.#words;
  }

  /** Allocate `len` bytes inside the module. Release with `free` and the same length. */
  alloc(len: number): number {
    const ptr = this.x.ghostty_wasm_alloc(len);
    if (ptr === 0) throw new VtError(OUT_OF_MEMORY, "ghostty_wasm_alloc");
    return ptr;
  }

  /** Allocate `len` zeroed bytes inside the module. */
  allocZeroed(len: number): number {
    const ptr = this.alloc(len);
    this.u8().fill(0, ptr, ptr + len);
    return ptr;
  }

  free(ptr: number, len: number): void {
    this.x.ghostty_wasm_free(ptr, len);
  }

  /**
   * Create an opaque handle. `create` receives the address the constructor writes the
   * handle to and returns the constructor's result code.
   */
  newHandle(what: string, create: (out: number) => number): number {
    const result = create(this.#handleSlot);
    // Taking also clears the slot, so a failed constructor cannot leave a stale handle.
    const handle = this.x.ghostty_wasm_take_opaque(this.#handleSlot);
    if (result !== SUCCESS) throw new VtError(result, what);
    if (handle === 0) throw new VtError(OUT_OF_MEMORY, what);
    return handle;
  }

  /** Throw unless `result` is success. */
  check(result: number, what: string): void {
    if (result !== SUCCESS) throw new VtError(result, what);
  }

  /** The NUL-terminated string at `ptr`. */
  cString(ptr: number): string {
    const bytes = this.u8();
    let end = ptr;
    while (bytes[end] !== 0) end++;
    return this.#decoder.decode(bytes.subarray(ptr, end));
  }

  /** The UTF-8 string of `len` bytes at `ptr`. Invalid sequences become U+FFFD. */
  string(ptr: number, len: number): string {
    return len === 0 ? "" : this.#decoder.decode(this.u8().subarray(ptr, ptr + len));
  }

  /** A copy of `len` bytes at `ptr`, safe to keep after further calls. */
  copy(ptr: number, len: number): Uint8Array<ArrayBuffer> {
    return this.u8().slice(ptr, ptr + len);
  }

  /**
   * Copy bytes into the module and return their address.
   *
   * The buffer is shared by every caller and reused, so the address is good only until
   * the next `stage` or `stageText`. Calls into the module are synchronous and the
   * wrapper never stages from inside one, so there is no overlap.
   */
  stage(bytes: Uint8Array): number {
    const ptr = this.#stagingFor(bytes.length);
    this.u8().set(bytes, ptr);
    return ptr;
  }

  /** Encode text as UTF-8 into the module. Returns the address and the byte length. */
  stageText(text: string): { ptr: number; len: number } {
    // A UTF-16 code unit encodes to at most three bytes.
    const ptr = this.#stagingFor(text.length * 3);
    const { written } = this.#encoder.encodeInto(
      text,
      this.u8().subarray(ptr, ptr + text.length * 3),
    );
    return { ptr, len: written };
  }

  /**
   * `len` zeroed bytes for a value passed to or read from one call. It is the staging
   * buffer, so it is good only until the next `stage`, `stageText` or `scratch`.
   */
  scratch(len: number): number {
    const ptr = this.#stagingFor(len);
    this.u8().fill(0, ptr, ptr + len);
    return ptr;
  }

  /**
   * Run an encoder that writes into a buffer and reports the length.
   *
   * Every encoder of the library follows one convention: it is given a buffer, its
   * capacity and where to store the number of bytes written, and when the buffer is too
   * small it stores the size it needs and reports that it ran out of space. `call`
   * receives those three and returns the result code; it runs a second time with a
   * larger buffer if needed. The result is a copy.
   */
  encode(
    what: string,
    call: (buf: number, cap: number, written: number) => number,
  ): Uint8Array<ArrayBuffer> {
    if (this.#encoded === 0) {
      this.#encoded = this.alloc(ENCODE_MIN_BYTES);
      this.#encodedCap = ENCODE_MIN_BYTES;
    }
    let result = call(this.#encoded, this.#encodedCap, this.#written);
    if (result === OUT_OF_SPACE) {
      const needed = this.view().getUint32(this.#written, true);
      this.free(this.#encoded, this.#encodedCap);
      this.#encoded = 0;
      this.#encoded = this.alloc(needed);
      this.#encodedCap = needed;
      result = call(this.#encoded, this.#encodedCap, this.#written);
    }
    this.check(result, what);
    return this.copy(this.#encoded, this.view().getUint32(this.#written, true));
  }

  #stagingFor(len: number): number {
    if (len > this.#stagingCap) {
      if (this.#staging !== 0) this.free(this.#staging, this.#stagingCap);
      this.#staging = 0;
      this.#stagingCap = 0;
      const cap = Math.max(len, STAGING_MIN_BYTES);
      this.#staging = this.alloc(cap);
      this.#stagingCap = cap;
    }
    return this.#staging;
  }
}

/** An instantiated libghostty-vt module. One per page is enough; terminals share it. */
export class Vt {
  /** @internal The raw module, for the other classes of this package. */
  readonly core: Core;

  private constructor(core: Core) {
    this.core = core;
  }

  /** The library version the module reports, for example `"1.0.1"`. */
  get version(): string {
    return this.core.version;
  }

  /**
   * How much memory the module holds, in bytes. Every terminal of the module lives in
   * it, and it grows as they need more but is never given back.
   */
  get memoryBytes(): number {
    return this.core.x.memory.buffer.byteLength;
  }

  /** Instantiate the module from its bytes or from a fetch response. */
  static async instantiate(
    source: ArrayBuffer | Uint8Array | Response | Promise<Response>,
    options: VtOptions = {},
  ): Promise<Vt> {
    const isResponse = (s: unknown): s is Response | Promise<Response> =>
      (typeof Response !== "undefined" && s instanceof Response) || s instanceof Promise;

    // The module has no imports: it is freestanding.
    const { instance } = isResponse(source)
      ? await WebAssembly.instantiateStreaming(source, {})
      : await WebAssembly.instantiate(source as BufferSource, {});

    const missing = Object.keys(REQUIRED_EXPORTS).filter((name) => !(name in instance.exports));
    if (missing.length > 0) {
      throw new Error(`ghostty-vt.wasm lacks required exports: ${missing.join(", ")}`);
    }
    const core = new Core(instance.exports as unknown as Exports);
    installSystem(core, options);
    return new Vt(core);
  }

  /**
   * Create a terminal with the given grid size. Call `free()` on it when done.
   *
   * Without `options` the terminal runs with the library's defaults.
   */
  createTerminal(cols: number, rows: number, options?: TerminalOptions): Terminal {
    return new Terminal(this.core, cols, rows, options);
  }
}

/**
 * Give the module the services a freestanding build has no way to reach by itself.
 * They are settings of the whole module instance, not of one terminal.
 */
function installSystem(core: Core, options: VtOptions): void {
  const { manifest, trampoline, x } = core;
  const option = (name: string) => manifest.enumValue("GhosttySysOption", name);

  // Without a source of secure random bytes, every operation that needs entropy fails.
  const random = trampoline.install(
    manifest.callback("GhosttySysRandomSecureFn"),
    (_userdata: number, buf: number, len: number): number => {
      try {
        for (let done = 0; done < len; done += RANDOM_CHUNK_BYTES) {
          const end = buf + Math.min(len, done + RANDOM_CHUNK_BYTES);
          crypto.getRandomValues(core.u8().subarray(buf + done, end));
        }
        return 1;
      } catch {
        return 0;
      }
    },
  );
  core.check(x.ghostty_sys_set(option("RANDOM_SECURE"), random), "ghostty_sys_set");

  const sink = options.log;
  if (!sink) return;
  const levels = manifest.enumValues("GhosttySysLogLevel");
  const names = new Map<number, LogLevel>([
    [levels.ERROR ?? -1, "error"],
    [levels.WARNING ?? -1, "warning"],
    [levels.INFO ?? -1, "info"],
    [levels.DEBUG ?? -1, "debug"],
  ]);
  const log = trampoline.install(
    manifest.callback("GhosttySysLogFn"),
    (
      _userdata: number,
      level: number,
      scope: number,
      scopeLen: number,
      msg: number,
      msgLen: number,
    ) => {
      try {
        sink(names.get(level) ?? "info", core.string(scope, scopeLen), core.string(msg, msgLen));
      } catch {
        // A failing log sink must not unwind through the module.
      }
    },
  );
  core.check(x.ghostty_sys_set(option("LOG"), log), "ghostty_sys_set");
}
