/**
 * A terminal's look, in the three forms it is needed in.
 *
 * The look comes from the design tokens the core resolves: a theme with the user's
 * settings laid over. The same colours go to three places that must agree: the
 * terminal, which answers a program that asks what its colours are; the renderer, which
 * draws with them; and the session, which is told what the view looks like.
 */

import { rgba, type TerminalTheme } from "@lume/renderer";
import {
  DEFAULT_SCHEME,
  DEFAULT_TOKENS,
  terminalAppearance,
  type Rgba,
  type TerminalAppearance,
} from "@lume/tokens";
import type { Rgb, TerminalColors } from "@lume/vt";

import type { Palette } from "./transport/types.ts";

/** Everything about a terminal's look that does not come from the program in it. */
export type Appearance = TerminalAppearance;

/** The look of the built-in dark theme, which a page has before the core has told it more. */
export const DEFAULT_APPEARANCE: Appearance = terminalAppearance(DEFAULT_TOKENS, DEFAULT_SCHEME);

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

/** How bright a colour looks, from 0 for black to 1 for white. */
function luminance(color: Rgb): number {
  const linear = (channel: number) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b);
}

/** How well two colours can be told apart, from 1 for the same colour to 21. */
export function contrast(a: Rgb, b: Rgb): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (light + 0.05) / (dark + 0.05);
}

/**
 * What the renderer draws with. `fontSizePx` is the size in use, which is the look's
 * own unless the window's text has been made larger or smaller.
 */
export function rendererTheme(appearance: Appearance, fontSizePx: number): TerminalTheme {
  const pack = (color: Rgb) => rgba(color.r, color.g, color.b);
  const packAlpha = (color: Rgba) => rgba(color.r, color.g, color.b, color.a);
  return {
    font: {
      family: appearance.fontFamily,
      sizePx: fontSizePx,
      lineHeight: appearance.lineHeight,
      weight: appearance.fontWeight,
      boldWeight: appearance.boldWeight,
    },
    cursor: pack(appearance.cursor),
    cursorText: pack(appearance.cursorText),
    selectionBackground: pack(appearance.selectionBackground),
    selectionForeground: pack(appearance.selectionForeground),
    searchMatch: packAlpha(appearance.searchMatch),
    searchActive: packAlpha(appearance.searchActive),
    searchActiveText: pack(
      contrast(appearance.foreground, appearance.searchActive) >=
        contrast(appearance.background, appearance.searchActive)
        ? appearance.foreground
        : appearance.background,
    ),
    padding: appearance.padding,
  };
}

/** A colour as CSS writes it. */
export function cssColor(color: Rgb): string {
  const channel = (value: number) => value.toString(16).padStart(2, "0");
  return `#${channel(color.r)}${channel(color.g)}${channel(color.b)}`;
}
