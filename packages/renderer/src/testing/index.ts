/**
 * Helpers for testing the renderer and code that uses it: hand-built frames and
 * stand-ins for the browser-dependent parts.
 */

export {
  BACKGROUND,
  FIXED_FRAMES,
  FOREGROUND,
  fullRedrawFrame,
  makeFrame,
  markDrawn,
  PALETTE,
  putText,
  select,
  setCursor,
  touchRow,
  type CellStyle,
  type FixedFrame,
  type MutableFrame,
} from "./frames.ts";
export {
  FakeFonts,
  FakeRasteriser,
  RecordingBackend,
  RecordingSink,
  TEST_THEME,
  type PlaneUpload,
  type SinkUpload,
} from "./fakes.ts";
