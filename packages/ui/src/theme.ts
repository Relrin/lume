/**
 * How a terminal looks until the user can choose: the built-in colours and font.
 *
 * The same colours go to three places that must agree: the terminal, which answers a
 * program that asks what its colours are; the renderer, which draws with them; and the
 * session, which is told what the view looks like.
 */

import { rgba, type TerminalTheme } from "@lume/renderer";
import type { Rgb, TerminalColors } from "@lume/vt";

import type { ColorScheme, Palette } from "./transport/types.ts";

/** Everything about a terminal's look that does not come from the program in it. */
export interface Appearance {
  readonly scheme: ColorScheme;
  readonly foreground: Rgb;
  readonly background: Rgb;
  readonly cursor: Rgb;
  readonly selectionBackground: Rgb;
  /** The sixteen ANSI colours: eight normal, then eight bright. */
  readonly ansi: readonly Rgb[];
  /** A CSS font-family list, most preferred first. */
  readonly fontFamily: string;
  /** Space between the edge of the view and the grid, in CSS pixels. */
  readonly padding: number;
}

function hex(value: number): Rgb {
  return { r: (value >> 16) & 0xff, g: (value >> 8) & 0xff, b: value & 0xff };
}

/**
 * The built-in look. The fonts are ones the three platforms install themselves, so
 * nothing has to be shipped: the first that exists is used.
 */
export const DEFAULT_APPEARANCE: Appearance = {
  scheme: "dark",
  foreground: hex(0xdfe3ee),
  background: hex(0x14161d),
  cursor: hex(0xdfe3ee),
  selectionBackground: hex(0x2f4f8f),
  ansi: [
    0x1c1f2a, 0xf0908a, 0x7ee0a0, 0xe6c384, 0x7fb4f0, 0xc4a0f0, 0x6fd3d6, 0xc5cada, 0x596079,
    0xff9d94, 0x98ecb4, 0xf2d49b, 0x9cc7ff, 0xd6b8ff, 0x8ce4e6, 0xffffff,
  ].map(hex),
  fontFamily: '"Cascadia Mono", Consolas, Menlo, "DejaVu Sans Mono", monospace',
  padding: 4,
};

/** The levels of the six-step colour cube that follows the ANSI colours in a palette. */
const CUBE_LEVELS = [0, 95, 135, 175, 215, 255];

/**
 * All 256 palette entries: the sixteen ANSI colours, the 6×6×6 colour cube and the grey
 * ramp. The cube and the ramp are the same in every terminal.
 */
export function palette256(ansi: readonly Rgb[]): Rgb[] {
  const palette = ansi.slice(0, 16);
  for (const r of CUBE_LEVELS) {
    for (const g of CUBE_LEVELS) {
      for (const b of CUBE_LEVELS) palette.push({ r, g, b });
    }
  }
  for (let step = 0; step < 24; step++) {
    const level = 8 + step * 10;
    palette.push({ r: level, g: level, b: level });
  }
  return palette;
}

/** The colours a terminal starts from and reports when a program asks. */
export function terminalColors(appearance: Appearance): TerminalColors {
  return {
    foreground: appearance.foreground,
    background: appearance.background,
    cursor: appearance.cursor,
    palette: palette256(appearance.ansi),
  };
}

/** The colours a session is told its view has. */
export function factsPalette(appearance: Appearance): Palette {
  return {
    fg: appearance.foreground,
    bg: appearance.background,
    cursor: appearance.cursor,
    ansi: appearance.ansi,
  };
}

/** What the renderer draws with, at a font size. */
export function rendererTheme(appearance: Appearance, fontSizePx: number): TerminalTheme {
  const pack = (color: Rgb) => rgba(color.r, color.g, color.b);
  return {
    font: { family: appearance.fontFamily, sizePx: fontSizePx },
    cursor: pack(appearance.cursor),
    selectionBackground: pack(appearance.selectionBackground),
    padding: appearance.padding,
  };
}

/** A colour as CSS writes it. */
export function cssColor(color: Rgb): string {
  return `rgb(${color.r} ${color.g} ${color.b})`;
}
