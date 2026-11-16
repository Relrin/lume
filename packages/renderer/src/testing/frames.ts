/**
 * Frames built by hand, without a terminal.
 *
 * The fixed set here exercises everything a renderer draws: plain and coloured text,
 * every attribute and underline style, double-width characters and emoji, box drawing,
 * each cursor shape, a selection, and a full screen of changing cells for timing. Tests
 * and the renderer lab draw the same set, so a tier is always judged on the same input.
 */

import {
  CellFlag,
  CursorShape,
  DirtyState,
  GLYPH_CLUSTER_TAG,
  HIGHLIGHT_STRIDE,
  HighlightKind,
  NO_SELECTION,
  rgba,
  UNDERLINE_SHIFT,
  UnderlineStyle,
  type Highlights,
  type RenderFrame,
} from "../frame.ts";

/** How a run of text looks. Colours are packed as a frame packs them. */
export interface CellStyle {
  fg?: number;
  bg?: number;
  underlineColor?: number;
  bold?: boolean;
  italic?: boolean;
  strikethrough?: boolean;
  overline?: boolean;
  /** An `UnderlineStyle`. */
  underline?: number;
  hyperlink?: boolean;
}

/** A frame whose cluster table can be added to. */
export type MutableFrame = RenderFrame & { clusters: string[] };

/** The sixteen standard colours, as most terminals ship them. */
export const PALETTE: readonly number[] = [
  rgba(0x1d, 0x1f, 0x21),
  rgba(0xcc, 0x66, 0x66),
  rgba(0xb5, 0xbd, 0x68),
  rgba(0xf0, 0xc6, 0x74),
  rgba(0x81, 0xa2, 0xbe),
  rgba(0xb2, 0x94, 0xbb),
  rgba(0x8a, 0xbe, 0xb7),
  rgba(0xc5, 0xc8, 0xc6),
  rgba(0x66, 0x66, 0x66),
  rgba(0xd5, 0x4e, 0x53),
  rgba(0xb9, 0xca, 0x4a),
  rgba(0xe7, 0xc5, 0x47),
  rgba(0x7a, 0xa6, 0xda),
  rgba(0xc3, 0x97, 0xd8),
  rgba(0x70, 0xc0, 0xb1),
  rgba(0xea, 0xea, 0xea),
];

export const FOREGROUND = rgba(0xd8, 0xdc, 0xe0);
export const BACKGROUND = rgba(0x16, 0x18, 0x1c);

/** An empty frame of the given size, with everything flagged as changed. */
export function makeFrame(cols: number, rows: number): MutableFrame {
  const cells = cols * rows;
  return {
    cols,
    rows,
    dirty: DirtyState.Full,
    dirtyRows: new Uint8Array(rows).fill(1),
    glyph: new Uint32Array(cells),
    fg: new Uint32Array(cells).fill(FOREGROUND),
    bg: new Uint32Array(cells),
    underlineColor: new Uint32Array(cells).fill(FOREGROUND),
    flags: new Uint16Array(cells),
    selectionStart: new Int32Array(rows).fill(NO_SELECTION),
    selectionEnd: new Int32Array(rows),
    clusters: [],
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
    foreground: FOREGROUND,
    background: BACKGROUND,
  };
}

/** Whether a codepoint takes two cells. Enough of the rule for the text used here. */
function isWide(codepoint: number): boolean {
  return (
    (codepoint >= 0x1100 && codepoint <= 0x115f) ||
    (codepoint >= 0x2e80 && codepoint <= 0xa4cf) ||
    (codepoint >= 0xac00 && codepoint <= 0xd7a3) ||
    (codepoint >= 0xf900 && codepoint <= 0xfaff) ||
    (codepoint >= 0xff00 && codepoint <= 0xff60) ||
    (codepoint >= 0x1f300 && codepoint <= 0x1faff) ||
    (codepoint >= 0x20000 && codepoint <= 0x3fffd)
  );
}

const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/**
 * Write text into a row starting at `col`, one grapheme per cell, two for a wide one.
 * Returns the column after the text. Text past the end of the row is dropped.
 */
export function putText(
  frame: MutableFrame,
  col: number,
  row: number,
  text: string,
  style: CellStyle = {},
): number {
  const fg = style.fg ?? frame.foreground;
  const flags =
    (style.bold ? CellFlag.Bold : 0) |
    (style.italic ? CellFlag.Italic : 0) |
    (style.strikethrough ? CellFlag.Strikethrough : 0) |
    (style.overline ? CellFlag.Overline : 0) |
    (style.hyperlink ? CellFlag.Hyperlink : 0) |
    ((style.underline ?? UnderlineStyle.None) << UNDERLINE_SHIFT);

  for (const { segment } of segmenter.segment(text)) {
    const first = segment.codePointAt(0) ?? 0;
    const wide = isWide(first);
    if (col + (wide ? 2 : 1) > frame.cols) break;
    const index = row * frame.cols + col;

    const single = segment.length === String.fromCodePoint(first).length;
    if (single) {
      frame.glyph[index] = first === 0x20 ? 0 : first;
    } else {
      let cluster = frame.clusters.indexOf(segment);
      if (cluster < 0) cluster = frame.clusters.push(segment) - 1;
      frame.glyph[index] = (GLYPH_CLUSTER_TAG | cluster) >>> 0;
    }
    frame.fg[index] = fg;
    frame.bg[index] = style.bg ?? 0;
    frame.underlineColor[index] = style.underlineColor ?? fg;
    frame.flags[index] = flags | (wide ? CellFlag.Wide : 0);
    col++;

    if (wide) {
      // The second cell repeats the look of the first and has no glyph of its own.
      frame.glyph[index + 1] = 0;
      frame.fg[index + 1] = fg;
      frame.bg[index + 1] = style.bg ?? 0;
      frame.underlineColor[index + 1] = style.underlineColor ?? fg;
      frame.flags[index + 1] = (flags & ~(CellFlag.Bold | CellFlag.Italic)) | CellFlag.Spacer;
      col++;
    }
  }
  return col;
}

/** Select the columns `start` to `end`, inclusive, of a row. */
export function select(frame: RenderFrame, row: number, start: number, end: number): void {
  frame.selectionStart[row] = start;
  frame.selectionEnd[row] = end;
}

/**
 * A list of highlighted runs, each given as its row, first column, last column
 * (inclusive) and `HighlightKind`.
 */
export function highlights(
  runs: readonly (readonly [number, number, number, number])[],
): Highlights {
  const data = new Int32Array(runs.length * HIGHLIGHT_STRIDE);
  runs.forEach((run, index) => data.set(run, index * HIGHLIGHT_STRIDE));
  return { data, count: runs.length };
}

/** Show the cursor at a cell in a `CursorShape`. */
export function setCursor(frame: RenderFrame, x: number, y: number, shape: number): void {
  Object.assign(frame.cursor, { visible: true, x, y, shape });
}

/** Mark a frame as drawn, as its producer would after a draw. */
export function markDrawn(frame: RenderFrame): void {
  frame.dirty = DirtyState.None;
  frame.dirtyRows.fill(0);
}

/** Flag one row of a frame as changed. */
export function touchRow(frame: RenderFrame, row: number): void {
  frame.dirtyRows[row] = 1;
  if (frame.dirty === DirtyState.None) frame.dirty = DirtyState.Partial;
}

const COLS = 80;
const ROWS = 24;

function asciiColors(): MutableFrame {
  const frame = makeFrame(COLS, ROWS);
  putText(frame, 0, 0, "The quick brown fox jumps over the lazy dog. 0123456789");
  putText(frame, 0, 1, "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz");
  putText(frame, 0, 2, "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~");
  for (let i = 0; i < 16; i++) {
    const row = 4 + (i >> 3) * 2;
    const col = (i & 7) * 10;
    putText(frame, col, row, ` color${i} `.slice(0, 9), { fg: PALETTE[i]! });
    putText(frame, col, row + 1, ` color${i} `.slice(0, 9), {
      bg: PALETTE[i]!,
      fg: i === 0 || i === 8 ? PALETTE[15]! : PALETTE[0]!,
    });
  }
  // A truecolour ramp, one cell per step, in three channels.
  for (let col = 0; col < COLS; col++) {
    const level = Math.round((col * 255) / (COLS - 1));
    putText(frame, col, 9, " ", { bg: rgba(level, 0, 0) });
    putText(frame, col, 10, " ", { bg: rgba(0, level, 0) });
    putText(frame, col, 11, " ", { bg: rgba(0, 0, level) });
    putText(frame, col, 12, "x", { fg: rgba(level, 255 - level, 128) });
  }
  return frame;
}

function attributes(): MutableFrame {
  const frame = makeFrame(COLS, ROWS);
  putText(frame, 0, 0, "regular text");
  putText(frame, 0, 1, "bold text", { bold: true });
  putText(frame, 0, 2, "italic text", { italic: true });
  putText(frame, 0, 3, "bold italic text", { bold: true, italic: true });
  putText(frame, 0, 4, "strikethrough text", { strikethrough: true });
  putText(frame, 0, 5, "overline text", { overline: true });
  putText(frame, 0, 6, "strike and overline", { strikethrough: true, overline: true });
  const styles: [string, number][] = [
    ["single underline gjpqy", UnderlineStyle.Single],
    ["double underline gjpqy", UnderlineStyle.Double],
    ["curly underline gjpqy", UnderlineStyle.Curly],
    ["dotted underline gjpqy", UnderlineStyle.Dotted],
    ["dashed underline gjpqy", UnderlineStyle.Dashed],
  ];
  styles.forEach(([text, underline], i) => {
    putText(frame, 0, 8 + i, text, { underline });
    putText(frame, 30, 8 + i, text, { underline, underlineColor: PALETTE[9]!, fg: PALETTE[12]! });
  });
  putText(frame, 0, 14, "underlined on a background", {
    underline: UnderlineStyle.Curly,
    bg: PALETTE[4]!,
    fg: PALETTE[15]!,
  });
  return frame;
}

function wideAndEmoji(): MutableFrame {
  const frame = makeFrame(COLS, ROWS);
  putText(frame, 0, 0, "日本語のテキスト 中文文本 한국어 텍스트");
  putText(frame, 0, 1, "mixed 半角 and 全角 text");
  putText(frame, 0, 2, "wide on colour: ", {});
  putText(frame, 16, 2, "漢字", { bg: PALETTE[5]!, fg: PALETTE[0]! });
  // The last two are black and grey: colour glyphs that a tint would ruin.
  putText(frame, 0, 4, "😀 😎 🚀 🌈 🎉 🖤 🏁");
  putText(frame, 0, 5, "👨‍👩‍👧 👍🏽 🇯🇵 🏳️‍🌈");
  putText(frame, 0, 7, "combining: é ä ô ñ Z͑ͫ̓");
  putText(frame, 0, 8, "underlined 漢字 text", { underline: UnderlineStyle.Single });
  return frame;
}

function boxDrawing(): MutableFrame {
  const frame = makeFrame(COLS, ROWS);
  const boxes = [
    ["┌──┬──┐", "│  │  │", "├──┼──┤", "│  │  │", "└──┴──┘"],
    ["┏━━┳━━┓", "┃  ┃  ┃", "┣━━╋━━┫", "┃  ┃  ┃", "┗━━┻━━┛"],
    ["╔══╦══╗", "║  ║  ║", "╠══╬══╣", "║  ║  ║", "╚══╩══╝"],
    ["╭──────╮", "│      │", "│      │", "│      │", "╰──────╯"],
    ["╒══╤══╕", "│  │  │", "╞══╪══╡", "│  │  │", "╘══╧══╛"],
    ["╓──╥──╖", "║  ║  ║", "╟──╫──╢", "║  ║  ║", "╙──╨──╜"],
  ];
  boxes.forEach((lines, box) => {
    lines.forEach((line, row) => putText(frame, box * 10, row, line));
  });
  putText(frame, 0, 6, "┍┑┎┒┕┙┖┚┝┥┞┦┟┧┠┨┡┩┢┪┭┮┯┰┱┲┵┶┷┸┹┺┽┾┿╀╁╂╃╄╅╆╇╈╉╊");
  putText(frame, 0, 7, "┄┅┆┇┈┉┊┋╌╍╎╏ ╱╲╳ ╴╵╶╷╸╹╺╻╼╽╾╿");
  putText(frame, 0, 9, "▀▁▂▃▄▅▆▇█▉▊▋▌▍▎▏▐░▒▓▔▕▖▗▘▙▚▛▜▝▞▟");
  for (let col = 0; col < 40; col++) {
    putText(frame, col, 11, "█", { fg: PALETTE[1 + (col % 6)]! });
    putText(frame, col, 12, "▄", { fg: PALETTE[9 + (col % 6)]!, bg: PALETTE[1 + (col % 6)]! });
  }
  return frame;
}

function cursorFrame(shape: number): MutableFrame {
  const frame = makeFrame(COLS, ROWS);
  putText(frame, 0, 0, "the cursor sits on the letter Q below");
  putText(frame, 0, 1, "  Q  ", {});
  putText(frame, 0, 3, "and here on a wide character:");
  putText(frame, 0, 4, "  漢  ");
  setCursor(frame, 2, 1, shape);
  return frame;
}

function selection(): MutableFrame {
  const frame = makeFrame(COLS, ROWS);
  putText(frame, 0, 0, "a selection that starts in the middle of this line");
  putText(frame, 0, 1, "covers the whole of this one, 漢字 included,", { fg: PALETTE[2]! });
  putText(frame, 0, 2, "and ends partway through the third.", { underline: UnderlineStyle.Single });
  select(frame, 0, 12, COLS - 1);
  select(frame, 1, 0, COLS - 1);
  select(frame, 2, 0, 18);
  return frame;
}

/** Text in which the word "match" was searched for, on plain and coloured cells. */
function searched(): MutableFrame {
  const frame = makeFrame(COLS, ROWS);
  putText(frame, 0, 0, "the first match is on plain text, and a match follows it");
  putText(frame, 0, 1, "a match on colour", { bg: PALETTE[4]!, fg: PALETTE[15]! });
  putText(frame, 0, 2, "a match in a selection", { fg: PALETTE[2]! });
  putText(frame, 10, 3, "one that wraps at the end of this row and goes on to the next one: mat");
  putText(frame, 0, 4, "ch, underlined", { underline: UnderlineStyle.Single });
  putText(frame, 0, 6, "no highlight on this row at all");
  select(frame, 2, 0, 9);
  return frame;
}

/** The runs of "match" in `searched`, with the one in row `active` being the one a search is at. */
function searchedRuns(active: number): Highlights {
  const kind = (row: number) => (row === active ? HighlightKind.Active : HighlightKind.Match);
  return highlights([
    [0, 10, 14, kind(0)],
    [0, 40, 44, HighlightKind.Match],
    [1, 2, 6, kind(1)],
    [2, 2, 6, kind(2)],
    [3, 77, 79, kind(3)],
    [4, 0, 1, kind(3)],
  ]);
}

/**
 * A full screen in which every cell holds a glyph that depends on `variant`, so drawing
 * two variants in turn changes every cell. Colours change every few cells, as they do
 * in real output, not at every cell.
 */
export function fullRedrawFrame(variant: number, cols = 200, rows = 60): MutableFrame {
  const frame = makeFrame(cols, rows);
  for (let row = 0; row < rows; row++) {
    for (let col = 0; col < cols; col++) {
      const index = row * cols + col;
      const n = (row * 31 + col * 17 + variant * 7) % 94;
      frame.glyph[index] = 0x21 + n;
      frame.fg[index] = PALETTE[1 + ((row + (col >> 3) + variant) % 15)]!;
      frame.bg[index] = ((col >> 4) + row + variant) % 5 === 0 ? PALETTE[8]! : 0;
      frame.flags[index] =
        (n % 7 === 0 ? CellFlag.Bold : 0) |
        (n % 11 === 0 ? CellFlag.Italic : 0) |
        (n % 13 === 0 ? UnderlineStyle.Single << UNDERLINE_SHIFT : 0);
      frame.underlineColor[index] = frame.fg[index]!;
    }
  }
  return frame;
}

/** A named frame of the fixed set. */
export interface FixedFrame {
  name: string;
  build(): MutableFrame;
  /** The runs to highlight when drawing it, if it has any. */
  highlights?(): Highlights;
}

/** The fixed frame set. */
export const FIXED_FRAMES: readonly FixedFrame[] = [
  { name: "ascii-colors", build: asciiColors },
  { name: "attributes", build: attributes },
  { name: "wide-emoji", build: wideAndEmoji },
  { name: "box-drawing", build: boxDrawing },
  { name: "cursor-block", build: () => cursorFrame(CursorShape.Block) },
  { name: "cursor-bar", build: () => cursorFrame(CursorShape.Bar) },
  { name: "cursor-underline", build: () => cursorFrame(CursorShape.Underline) },
  { name: "cursor-hollow", build: () => cursorFrame(CursorShape.HollowBlock) },
  { name: "selection", build: selection },
  { name: "search-matches", build: searched, highlights: () => searchedRuns(-1) },
  { name: "search-active", build: searched, highlights: () => searchedRuns(3) },
  { name: "full-200x60", build: () => fullRedrawFrame(0) },
];
