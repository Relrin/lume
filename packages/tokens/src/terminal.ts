/**
 * The tokens a terminal is drawn with, in the form a terminal takes them.
 *
 * A terminal is not styled with CSS: its colours go to the engine, which tells a
 * program what they are when it asks, and to the renderer, which draws with them. Both
 * want numbers, not text, so the colours are read here once.
 */

import { cssFontStack } from "./css.ts";
import type { Tokens } from "./generated.ts";

/** A colour as three channels, each 0 to 255. */
export interface Rgb {
  readonly r: number;
  readonly g: number;
  readonly b: number;
}

/** A colour with its opacity, 0 to 255. */
export interface Rgba extends Rgb {
  readonly a: number;
}

/** Whether the look is a light or a dark one. */
export type Scheme = "light" | "dark";

const HEX = /^#([0-9a-f]{6})([0-9a-f]{2})?$/i;

/**
 * Reads a colour written as `#rrggbb` or `#rrggbbaa`. The application checks every
 * token before sending it, so anything else here is a fault and is thrown.
 */
export function parseColor(text: string): Rgba {
  const match = HEX.exec(text);
  const digits = match?.[1];
  if (digits === undefined) throw new Error(`${JSON.stringify(text)} is not a colour`);
  const value = Number.parseInt(digits, 16);
  const alpha = match?.[2];
  return {
    r: (value >> 16) & 0xff,
    g: (value >> 8) & 0xff,
    b: value & 0xff,
    a: alpha === undefined ? 255 : Number.parseInt(alpha, 16),
  };
}

/** The colour without its opacity. */
function opaque(color: Rgba): Rgb {
  return { r: color.r, g: color.g, b: color.b };
}

/** Everything about a terminal's look that does not come from the program in it. */
export interface TerminalAppearance {
  readonly scheme: Scheme;
  readonly foreground: Rgb;
  readonly background: Rgb;
  readonly cursor: Rgb;
  /** The character under a block cursor. */
  readonly cursorText: Rgb;
  readonly selectionBackground: Rgb;
  readonly selectionForeground: Rgb;
  /** The background of text a search found, and of the match the search is at. */
  readonly searchMatch: Rgba;
  readonly searchActive: Rgba;
  /** The sixteen ANSI colours: eight normal, then eight bright. */
  readonly ansi: readonly Rgb[];
  /** A CSS font-family list, most preferred first. */
  readonly fontFamily: string;
  readonly fontSizePx: number;
  /** The height of a line as a multiple of the font's size. */
  readonly lineHeight: number;
  readonly fontWeight: number;
  readonly boldWeight: number;
  /** Space between the edge of the view and the grid, in CSS pixels. */
  readonly padding: number;
}

/** How many ANSI colours a palette has. */
const ANSI_COLORS = 16;

/** The terminal's look under `tokens`. `scheme` says whether the theme is light or dark. */
export function terminalAppearance(tokens: Tokens, scheme: Scheme): TerminalAppearance {
  const ansi: Rgb[] = [];
  for (let index = 0; index < ANSI_COLORS; index++) {
    const name = `terminal.palette.${index}` as keyof Tokens;
    ansi.push(opaque(parseColor(String(tokens[name]))));
  }
  return {
    scheme,
    foreground: opaque(parseColor(tokens["terminal.fg"])),
    background: opaque(parseColor(tokens["terminal.bg"])),
    cursor: opaque(parseColor(tokens["terminal.cursor"])),
    cursorText: opaque(parseColor(tokens["terminal.cursor-text"])),
    selectionBackground: opaque(parseColor(tokens["terminal.selection"])),
    selectionForeground: opaque(parseColor(tokens["terminal.selection-text"])),
    searchMatch: parseColor(tokens["terminal.search-match"]),
    searchActive: parseColor(tokens["terminal.search-active"]),
    ansi,
    fontFamily: cssFontStack(tokens["font.family"], tokens["font.fallback"]),
    fontSizePx: tokens["font.size"],
    lineHeight: tokens["font.line-height"],
    fontWeight: tokens["font.weight"],
    boldWeight: tokens["font.bold-weight"],
    padding: tokens["space.padding"],
  };
}
