/**
 * Status items: small pieces of information shown in the window's bars.
 *
 * An item is plain data. Whoever has something to show registers an item and updates
 * it; the window draws every item of a kind the same way, in the place the item names.
 * The shapes here are the ones a plugin sends: an item carries text and numbers, never
 * markup, and an icon is one of a fixed set of names.
 */

import type { PaneId } from "../layout/types.ts";

/** Where status items are shown: the three parts of the top bar, and a pane's header. */
export type StatusSlot = "topbar.left" | "topbar.center" | "topbar.right" | "pane.header";

/** The slots there are. */
export const STATUS_SLOTS = [
  "topbar.left",
  "topbar.center",
  "topbar.right",
  "pane.header",
] as const satisfies readonly StatusSlot[];

/** What an item's colour says about what it shows. */
export type StatusTone = "neutral" | "info" | "success" | "warning" | "danger";

/** The icons an item may show, by name. */
export type StatusIconName = "terminal" | "branch" | "pulse" | "folder" | "dot" | "alert";

/** What every item has. */
interface StatusBase {
  /** Names the item: dotted and lower case, beginning with its owner's id. */
  readonly id: string;
  readonly slot: StatusSlot;
  /** Items of a slot are shown in rising order; items alike in this, by id. */
  readonly order?: number;
  readonly tone?: StatusTone;
  /** What the pointer is told when it rests on the item. */
  readonly tooltip?: string;
  /** The pane whose header shows the item, for an item of the `pane.header` slot. */
  readonly pane?: PaneId;
}

/** A few words. */
export interface TextItem extends StatusBase {
  readonly kind: "text";
  readonly text: string;
  /** Whether the text is drawn in the terminal's font, as a path or a number is. */
  readonly mono?: boolean;
}

/** An icon, with the words that say what it stands for. */
export interface IconItem extends StatusBase {
  readonly kind: "icon";
  readonly icon: StatusIconName;
  /** Read out in place of the icon, and shown beside it when `show_label` is set. */
  readonly label: string;
  readonly show_label?: boolean;
}

/** How full something is: a name, a bar and the amount in words. */
export interface MeterItem extends StatusBase {
  readonly kind: "meter";
  readonly label: string;
  /** From nought, which is empty, to one, which is full. */
  readonly value: number;
  /** The amount written out, as in `31%`. */
  readonly text?: string;
}

/** How something went lately: a small line through the latest values. */
export interface SparklineItem extends StatusBase {
  readonly kind: "sparkline";
  readonly label: string;
  /** The values from the oldest to the newest. They are drawn between their own extremes. */
  readonly points: readonly number[];
  /** The latest value written out. */
  readonly text?: string;
}

/** A count or a word in a small box. */
export interface BadgeItem extends StatusBase {
  readonly kind: "badge";
  readonly text: string;
}

/** One status item. */
export type StatusItem = TextItem | IconItem | MeterItem | SparklineItem | BadgeItem;

/** The kinds of status item there are. */
export const STATUS_KINDS = [
  "text",
  "icon",
  "meter",
  "sparkline",
  "badge",
] as const satisfies readonly StatusItem["kind"][];
