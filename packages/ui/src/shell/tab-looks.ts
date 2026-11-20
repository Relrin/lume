/**
 * What the colours and pictures tabs and groups can be given are called, and the
 * theme's colour for each.
 */

import type { CSSProperties } from "react";

import { startingHex, tokenHex } from "./color-picker.ts";

import type { GroupIcon, NamedTabColor, TabColor } from "../layout/types.ts";

/** What each named colour is called. */
export const TAB_COLOR_NAMES: Readonly<Record<NamedTabColor, string>> = {
  red: "Red",
  orange: "Orange",
  yellow: "Yellow",
  green: "Green",
  teal: "Teal",
  blue: "Blue",
  purple: "Purple",
  pink: "Pink",
};

/** What each picture of a group stands for. */
export const GROUP_ICON_NAMES: Readonly<Record<GroupIcon, string>> = {
  bug: "Bug",
  flame: "Incident",
  rocket: "Release",
  flask: "Experiment",
  wrench: "Maintenance",
  layers: "Stack",
  server: "Infrastructure",
  book: "Docs",
};

/** Whether a colour is one of the user's own, written as `#rrggbb`. */
export function isOwnColor(color: TabColor): color is `#${string}` {
  return color.startsWith("#");
}

/** A colour as the theme has it, or as the user mixed it, for a style. */
export function tabColorValue(color: TabColor): string {
  return isOwnColor(color) ? color : `var(--lume-color-tab-${color})`;
}

/**
 * A colour as `#rrggbb`, for a picker to start from: the user's own as it is, a named
 * one as the theme in use has it.
 */
export function themeHex(color: TabColor): string {
  if (isOwnColor(color)) return color;
  return tokenHex(`--lume-color-tab-${color}`) ?? startingHex();
}

/**
 * The style that gives an element a colour of the user's own under `variable`, which
 * the style sheet reads where a named colour has a rule of its own. Nothing for a
 * named colour, or for none.
 */
export function ownColorStyle(
  variable: "--tab-tint" | "--group-color",
  color: TabColor | null | undefined,
): CSSProperties | undefined {
  if (color === null || color === undefined || !isOwnColor(color)) return undefined;
  return { [variable]: color } as CSSProperties;
}

/** What a group is called where it has no name of its own. */
export const UNNAMED_GROUP = "Unnamed group";

/** How many tabs, in words. */
export function tabCount(count: number): string {
  return `${count} tab${count === 1 ? "" : "s"}`;
}
