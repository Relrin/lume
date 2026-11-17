/**
 * Where a tab or a group being dragged along the row of tabs would land, and what
 * letting it go there asks of the layout.
 *
 * A tab let go between two tabs of a group joins the group; let go just outside a
 * group's first or last tab, it stays out of it, so a tab can be put beside a group
 * without joining it. Let go on a group's label, it joins at the group's start. A
 * group is dragged by its label and moves whole: what it is let go over counts as a
 * whole group too, so a group never lands inside another.
 *
 * Nothing here reads the page: the row is given as plain boxes and plain lists, so
 * the rules are tested as they are.
 */

import type { GroupId, Intent, TabId } from "../layout/types.ts";
import { groupOfLabel, labelId, type DropSide } from "./tab-ui.ts";

/** One thing in the row as it is laid out along it. */
export interface DropBox {
  /** A tab's id, or a group's label as the row names it. */
  readonly id: string;
  /** The group it belongs to, or is the label of. */
  readonly group: GroupId | null;
  /** Where it starts and ends along the row. */
  readonly start: number;
  readonly end: number;
  /** Whether it is folded away with its group. A folded tab is no place to land. */
  readonly folded: boolean;
}

/** One tab of the row, in order, and its group. */
export interface RowTab {
  readonly tab: TabId;
  readonly group: GroupId | null;
}

/** Where a drag would land. */
export interface DropPlace {
  readonly target: string;
  readonly side: DropSide;
}

/** The box at a point along the row, if the point is on one. */
function boxAt(boxes: readonly DropBox[], at: number): DropBox | undefined {
  return boxes.find((box) => !box.folded && at >= box.start && at < box.end);
}

/**
 * Where what is dragged would land with the pointer at `at` along the row. `source`
 * names what is dragged as the row does. `undefined` means the place is not another
 * one: the pointer is between two boxes, or over the group being dragged, and the place
 * found before stands.
 */
export function dropPlace(
  boxes: readonly DropBox[],
  source: string,
  at: number,
): DropPlace | undefined {
  const hit = boxAt(boxes, at);
  if (!hit) return undefined;
  const middle = (box: { start: number; end: number }) => (box.start + box.end) / 2;
  const dragged = groupOfLabel(source);
  if (dragged === null) {
    if (hit.id === source) return { target: source, side: "before" };
    // A tab let go on a label joins that group at its start.
    if (groupOfLabel(hit.id) !== null) return { target: hit.id, side: "after" };
    return { target: hit.id, side: at < middle(hit) ? "before" : "after" };
  }
  if (hit.group === dragged) return undefined;
  if (hit.group === null) return { target: hit.id, side: at < middle(hit) ? "before" : "after" };
  // Another group: before its label or after its last tab, judged against the whole of it.
  const block = boxes.filter((box) => box.group === hit.group);
  const label = block.find((box) => box.id === labelId(hit.group ?? ""));
  const open = block.filter((box) => !box.folded);
  const start = Math.min(...open.map((box) => box.start));
  const end = Math.max(...open.map((box) => box.end));
  const before = at < (start + end) / 2;
  const last = open.at(-1);
  if (before) return { target: label?.id ?? hit.id, side: "before" };
  // A folded group's last tab is out of sight: the label stands for the whole group.
  return { target: last?.id ?? hit.id, side: "after" };
}

/**
 * The group a tab let go beside `target` joins. Beside a tab of a group it joins that
 * group, unless it is let go just outside the group's first or last tab; it stays in
 * its own group anywhere among that group's tabs.
 */
export function joinedGroup(
  row: readonly RowTab[],
  moving: TabId,
  target: string,
  side: DropSide,
): GroupId | null {
  const onLabel = groupOfLabel(target);
  if (onLabel !== null) return onLabel;
  const dst = row.find((entry) => entry.tab === target);
  const group = dst?.group ?? null;
  if (group === null || !dst) return null;
  const own = row.find((entry) => entry.tab === moving)?.group ?? null;
  if (own === group) return group;
  const members = row.filter((entry) => entry.group === group && entry.tab !== moving);
  const outerEdge =
    (side === "before" && members[0]?.tab === dst.tab) ||
    (side === "after" && members.at(-1)?.tab === dst.tab);
  return outerEdge ? null : group;
}

/**
 * What letting a tab go at a place asks of the layout: to be put at a place among the
 * tabs and in a group, or in none. Nothing when it would land where it is.
 */
export function tabDropIntent(
  row: readonly RowTab[],
  moving: TabId,
  place: DropPlace,
): Extract<Intent, { type: "tab_place" }> | null {
  const from = row.findIndex((entry) => entry.tab === moving);
  if (from < 0 || place.target === moving) return null;
  const rest = row.filter((entry) => entry.tab !== moving);
  const group = joinedGroup(row, moving, place.target, place.side);
  const onLabel = groupOfLabel(place.target);
  let index: number;
  if (onLabel !== null) {
    index = rest.findIndex((entry) => entry.group === onLabel);
    if (index < 0) return null;
  } else {
    const at = rest.findIndex((entry) => entry.tab === place.target);
    if (at < 0) return null;
    index = at + (place.side === "after" ? 1 : 0);
  }
  const own = row[from]?.group ?? null;
  if (index === from && group === own) return null;
  return { type: "tab_place", tab: moving, index, group };
}

/**
 * What letting a group go at a place asks of the layout: its tabs moved together to a
 * place among the others. Nothing when it would land where it is, or inside another
 * group.
 */
export function groupDropIntent(
  row: readonly RowTab[],
  group: GroupId,
  place: DropPlace,
): Extract<Intent, { type: "group_move" }> | null {
  const block = row.filter((entry) => entry.group === group);
  if (block.length === 0) return null;
  const rest = row.filter((entry) => entry.group !== group);
  const from = row.findIndex((entry) => entry.group === group);
  const onLabel = groupOfLabel(place.target);
  let index: number;
  if (onLabel !== null) {
    if (onLabel === group) return null;
    const members = rest
      .map((entry, at) => ({ entry, at }))
      .filter(({ entry }) => entry.group === onLabel);
    const first = members[0];
    const last = members.at(-1);
    if (!first || !last) return null;
    index = place.side === "after" ? last.at + 1 : first.at;
  } else {
    const at = rest.findIndex((entry) => entry.tab === place.target);
    if (at < 0) return null;
    index = at + (place.side === "after" ? 1 : 0);
  }
  const before = rest[index - 1]?.group ?? null;
  const after = rest[index]?.group ?? null;
  if (before !== null && before === after) return null;
  if (index === from) return null;
  return { type: "group_move", group, index };
}
