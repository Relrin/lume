/**
 * The lab's frames that come from the terminal engine.
 *
 * Each is program output fed to the WebAssembly terminal, whose render state then
 * produces the frame. They show the same things as the hand-built frames of the
 * renderer's fixed set, by the path real output takes.
 */

import {
  clearSelection,
  RenderState,
  selectAll,
  type RenderFrame,
  type Rgb,
  type Terminal,
  type Vt,
} from "@lume/vt";
import { BACKGROUND, FOREGROUND, PALETTE } from "@lume/renderer/testing";

const ESC = "\x1b";
const CSI = `${ESC}[`;

/** A frame source backed by a terminal. Free it when done. */
export interface TerminalFrame {
  frame: RenderFrame;
  /** Feed more output to the terminal. */
  write(data: string): void;
  /** Bring the frame up to date with the terminal. */
  update(): RenderFrame;
  markDrawn(): void;
  free(): void;
}

function unpack(color: number): Rgb {
  return { r: color & 0xff, g: (color >>> 8) & 0xff, b: (color >>> 16) & 0xff };
}

/** The 256-colour palette: the lab's sixteen, then the standard cube and grey ramp. */
function palette(): Rgb[] {
  const colors = PALETTE.map(unpack);
  const steps = [0, 95, 135, 175, 215, 255];
  for (let i = 0; i < 216; i++) {
    colors.push({
      r: steps[Math.floor(i / 36)]!,
      g: steps[Math.floor(i / 6) % 6]!,
      b: steps[i % 6]!,
    });
  }
  for (let i = 0; i < 24; i++) {
    const level = 8 + i * 10;
    colors.push({ r: level, g: level, b: level });
  }
  return colors;
}

function open(vt: Vt, cols: number, rows: number): { term: Terminal; state: RenderState } {
  const term = vt.createTerminal(cols, rows, {
    colors: { foreground: unpack(FOREGROUND), background: unpack(BACKGROUND), palette: palette() },
    // The lab keeps no history worth speaking of.
    scrollbackMaxLines: 100,
    scrollbackMaxBytes: 1 << 20,
  });
  return { term, state: new RenderState(term) };
}

function source(
  vt: Vt,
  cols: number,
  rows: number,
  output: string,
  after?: (term: Terminal) => void,
) {
  return (): TerminalFrame => {
    const { term, state } = open(vt, cols, rows);
    term.write(output);
    after?.(term);
    return {
      frame: state.update(),
      write: (data) => term.write(data),
      update: () => state.update(),
      markDrawn: () => state.markDrawn(),
      free() {
        state.free();
        term.free();
      },
    };
  };
}

const HIDE_CURSOR = `${CSI}?25l`;

const asciiColors = [
  "The quick brown fox jumps over the lazy dog. 0123456789\r\n",
  "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz\r\n",
  "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~\r\n\r\n",
  ...[0, 8].flatMap((base) => [
    Array.from({ length: 8 }, (_, i) =>
      `${CSI}38;5;${base + i}m color${base + i}  `.slice(0, 20),
    ).join("") + `${CSI}0m\r\n`,
    Array.from(
      { length: 8 },
      (_, i) =>
        `${CSI}48;5;${base + i};38;5;${base + i === 0 || base + i === 8 ? 15 : 0}m color${base + i} ${CSI}0m `,
    ).join("") + "\r\n",
  ]),
  "\r\n",
  Array.from({ length: 80 }, (_, i) => `${CSI}48;2;${Math.round((i * 255) / 79)};0;0m `).join("") +
    `${CSI}0m\r\n`,
  Array.from({ length: 80 }, (_, i) => `${CSI}48;2;0;${Math.round((i * 255) / 79)};0m `).join("") +
    `${CSI}0m\r\n`,
  Array.from({ length: 80 }, (_, i) => `${CSI}48;2;0;0;${Math.round((i * 255) / 79)}m `).join("") +
    `${CSI}0m`,
  HIDE_CURSOR,
].join("");

const attributes = [
  "regular text\r\n",
  `${CSI}1mbold text${CSI}0m\r\n`,
  `${CSI}3mitalic text${CSI}0m\r\n`,
  `${CSI}1;3mbold italic text${CSI}0m\r\n`,
  `${CSI}2mfaint text${CSI}0m\r\n`,
  `${CSI}7mreverse video${CSI}0m ${CSI}7;31mreverse red${CSI}0m\r\n`,
  `${CSI}9mstrikethrough text${CSI}0m\r\n`,
  `${CSI}53moverline text${CSI}0m\r\n`,
  `${CSI}8mconcealed${CSI}0m <- concealed text is here\r\n\r\n`,
  ...[
    ["single", "4"],
    ["double", "4:2"],
    ["curly", "4:3"],
    ["dotted", "4:4"],
    ["dashed", "4:5"],
  ].map(
    ([name, code]) =>
      `${CSI}${code}m${name} underline gjpqy${CSI}0m   ` +
      `${CSI}${code};58;5;9;38;5;12m${name} underline gjpqy${CSI}0m\r\n`,
  ),
  `\r\n${CSI}4:3;44;97munderlined on a background${CSI}0m`,
  `\r\n${ESC}]8;;https://example.com${ESC}\\a hyperlink${ESC}]8;;${ESC}\\ and plain text`,
  HIDE_CURSOR,
].join("");

const wideEmoji = [
  "日本語のテキスト 中文文本 한국어 텍스트\r\n",
  "mixed 半角 and 全角 text\r\n",
  `wide on colour: ${CSI}45;30m漢字${CSI}0m\r\n\r\n`,
  // Grapheme clustering, so that joined emoji occupy one cell pair.
  `${CSI}?2027h😀 😎 🚀 🌈 🎉 🖤 🏁\r\n`,
  "👨‍👩‍👧 👍🏽 🇯🇵 🏳️‍🌈\r\n\r\n",
  "combining: é ä ô ñ\r\n",
  `${CSI}4munderlined 漢字 text${CSI}0m`,
  HIDE_CURSOR,
].join("");

const boxes = [
  ["┌──┬──┐", "│  │  │", "├──┼──┤", "│  │  │", "└──┴──┘"],
  ["┏━━┳━━┓", "┃  ┃  ┃", "┣━━╋━━┫", "┃  ┃  ┃", "┗━━┻━━┛"],
  ["╔══╦══╗", "║  ║  ║", "╠══╬══╣", "║  ║  ║", "╚══╩══╝"],
  ["╭──────╮", "│      │", "│      │", "│      │", "╰──────╯"],
  ["╒══╤══╕", "│  │  │", "╞══╪══╡", "│  │  │", "╘══╧══╛"],
  ["╓──╥──╖", "║  ║  ║", "╟──╫──╢", "║  ║  ║", "╙──╨──╜"],
];

const boxDrawing = [
  ...Array.from(
    { length: 5 },
    (_, row) => boxes.map((box) => box[row]!.padEnd(10)).join("") + "\r\n",
  ),
  "\r\n┍┑┎┒┕┙┖┚┝┥┞┦┟┧┠┨┡┩┢┪┭┮┯┰┱┲┵┶┷┸┹┺┽┾┿╀╁╂╃╄╅╆╇╈╉╊\r\n",
  "┄┅┆┇┈┉┊┋╌╍╎╏ ╱╲╳ ╴╵╶╷╸╹╺╻╼╽╾╿\r\n\r\n",
  "▀▁▂▃▄▅▆▇█▉▊▋▌▍▎▏▐░▒▓▔▕▖▗▘▙▚▛▜▝▞▟\r\n\r\n",
  Array.from({ length: 40 }, (_, i) => `${CSI}38;5;${1 + (i % 6)}m█`).join("") + `${CSI}0m\r\n`,
  Array.from({ length: 40 }, (_, i) => `${CSI}38;5;${9 + (i % 6)};48;5;${1 + (i % 6)}m▄`).join("") +
    `${CSI}0m`,
  HIDE_CURSOR,
].join("");

function cursor(shape: number): string {
  return [
    "the cursor sits on the letter Q below\r\n",
    "  Q  \r\n\r\n",
    "and here on a wide character:\r\n",
    "  漢  ",
    // Cursor shape, then position on the Q.
    `${CSI}${shape} q${CSI}2;3H`,
  ].join("");
}

const selection = [
  "everything on this screen is selected,\r\n",
  `${CSI}32mincluding 漢字 and coloured text,${CSI}0m\r\n`,
  `${CSI}4mand underlined text.${CSI}0m`,
  HIDE_CURSOR,
].join("");

/** Output that fills a 200 by 60 screen with changing glyphs and colours. */
export function fullRedrawOutput(variant: number, cols = 200, rows = 60): string {
  let out = `${CSI}H`;
  for (let row = 0; row < rows; row++) {
    let color = -1;
    for (let col = 0; col < cols; col++) {
      const n = (row * 31 + col * 17 + variant * 7) % 94;
      const next = 1 + ((row + (col >> 3) + variant) % 15);
      if (next !== color) {
        out += `${CSI}38;5;${next}m`;
        color = next;
      }
      out += String.fromCharCode(0x21 + n);
    }
    if (row < rows - 1) out += "\r\n";
  }
  return out;
}

/** The terminal-fed frames, by name. */
export function terminalFrames(vt: Vt): Record<string, () => TerminalFrame> {
  return {
    "vt: ascii-colors": source(vt, 80, 24, asciiColors),
    "vt: attributes": source(vt, 80, 24, attributes),
    "vt: wide-emoji": source(vt, 80, 24, wideEmoji),
    "vt: box-drawing": source(vt, 80, 24, boxDrawing),
    "vt: cursor-block": source(vt, 80, 24, cursor(2)),
    "vt: cursor-bar": source(vt, 80, 24, cursor(6)),
    "vt: cursor-underline": source(vt, 80, 24, cursor(4)),
    "vt: selection": source(vt, 80, 24, selection, (term) => {
      clearSelection(term);
      selectAll(term);
    }),
    "vt: full-200x60": source(vt, 200, 60, fullRedrawOutput(0) + HIDE_CURSOR),
  };
}
