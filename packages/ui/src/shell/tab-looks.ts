/**
 * What the colours and pictures tabs and groups can be given are called, and the
 * theme's colour for each.
 */

import type { GroupIcon, TabColor } from "../layout/types.ts";

/** What each colour is called. */
export const TAB_COLOR_NAMES: Readonly<Record<TabColor, string>> = {
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

/** A colour as the theme has it, for a style. */
export function tabColorValue(color: TabColor): string {
  return `var(--lume-color-tab-${color})`;
}

/** What a group is called where it has no name of its own. */
export const UNNAMED_GROUP = "Unnamed group";

/** How many tabs, in words. */
export function tabCount(count: number): string {
  return `${count} tab${count === 1 ? "" : "s"}`;
}
