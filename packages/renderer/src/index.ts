/**
 * Drawing a terminal's cells.
 *
 * One interface, `CellRenderer`, over three ways of drawing: WebGPU, WebGL2 and Canvas
 * 2D. The best one a webview supports is used, and a tier that is lost while running is
 * replaced by the next. All three draw from the same instance records and the same
 * glyph pixels, so they produce the same picture.
 *
 * A window creates one `RenderHost` and asks it for a renderer per pane.
 */

export { CURSOR_BAR_WIDTH_RATIO, TIER_ORDER, WEBGL2_CONTEXTS_MAX } from "./constants.ts";
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
export { RenderHost, type RenderHostOptions } from "./host.ts";
export {
  CursorEffects,
  type CursorEffectSettings,
  type FxMakers,
  type FxSurface,
} from "./fx/effects.ts";
export type { FxRect, MotionKind, PulseKind, TrailKind } from "./fx/geometry.ts";
export {
  CustomEffectError,
  customFragment,
  type CustomEffect,
  type CustomDraw,
} from "./fx/custom.ts";
export { Canvas2dFx } from "./fx/canvas2d.ts";
export { FxShape, ShapeKind, type FxBackend } from "./fx/shape.ts";
export { WebGl2Fx } from "./fx/webgl2.ts";
export {
  FrameLoop,
  type FrameLoopEnvironment,
  type FrameView,
  type FrameViewOptions,
} from "./loop.ts";
export { cellMetrics, gridSize } from "./metrics.ts";
export type { RenderStats } from "./pool.ts";
export { TierUnavailableError, type RendererOptions } from "./select.ts";
export type {
  CellMetrics,
  CellRenderer,
  FontSpec,
  RawFontMetrics,
  Size,
  TerminalTheme,
  Tier,
  UnderlineRange,
} from "./types.ts";
