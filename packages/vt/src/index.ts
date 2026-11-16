/**
 * A TypeScript wrapper over the WebAssembly build of libghostty-vt.
 *
 * It covers what a terminal view needs from the engine: feeding output and answering the
 * queries in it, reading the screen for drawing, encoding keys, mouse, focus and paste,
 * selection, scrollback and text capture. It mirrors the Rust `lume-vt` crate where both
 * exist, so the two builds can be checked against the same expectations.
 *
 * The wrapper uses only functions the unpatched upstream module exports. Struct layouts,
 * enum values and bit positions are not written out here: they are read from the module's
 * own `ghostty_type_json()`.
 */

export { VtError } from "./error.ts";
export { Vt, type LogLevel, type LogSink, type VtOptions } from "./module.ts";
export {
  Mode,
  Terminal,
  type ClipboardLocation,
  type ColorScheme,
  type Cursor,
  type CursorStyle,
  type OutputKind,
  type Rgb,
  type Scrollbar,
  type TerminalColors,
  type TerminalHandlers,
  type TerminalOptions,
} from "./terminal.ts";
export type { Attrs, Cell, Color, Wide } from "./cell.ts";
export {
  CellFlag,
  CursorShape,
  DirtyState,
  GLYPH_CLUSTER_TAG,
  HIGHLIGHT_STRIDE,
  HighlightKind,
  NO_SELECTION,
  rgba,
  UNDERLINE_MASK,
  UNDERLINE_SHIFT,
  UnderlineStyle,
  type FrameCursor,
  type Highlights,
  type RenderFrame,
} from "./frame.ts";
export { RenderState } from "./render.ts";
export { newHighlights, Search, type SearchStatus } from "./search.ts";
export { KeyEncoder, Mod, type KeyAction, type KeyInput } from "./keys.ts";
export {
  MouseEncoder,
  type MouseAction,
  type MouseButton,
  type MouseGeometry,
  type MouseInput,
} from "./mouse.ts";
export { encodeColorScheme, encodeFocus, encodePaste, isPasteSafe } from "./input.ts";
export {
  clearSelection,
  extendSelection,
  hasSelection,
  hyperlinkAt,
  selectAll,
  SelectionGesture,
  type Autoscroll,
  type ClickTiming,
  type PointerAt,
  type SelectionGeometry,
} from "./selection.ts";
export { formatText, selectionText, type FormatOptions, type TextFormat } from "./format.ts";
export {
  ANSI_MODES,
  DEC_MODES,
  dump,
  parseScript,
  replay,
  type Replayed,
  type Resize,
  type Script,
} from "./corpus.ts";
