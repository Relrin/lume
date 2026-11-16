/**
 * Moving the keyboard among the members of a group that takes one stop in the order
 * the Tab key goes through: the tabs of a tab list, the pages of a list of pages.
 *
 * Only one member of such a group can be reached with Tab. The arrow keys move from it
 * to its neighbours, and Home and End to the first and the last. The rule itself reads
 * nothing from the page, so it is tested with plain values.
 */

/** Which way the members of a group are laid out. */
export type RovingOrientation = "horizontal" | "vertical";

/**
 * The member a key moves the keyboard to, from the member at `index` among `count`, or
 * nothing when the key does not move it. The arrows along the group's direction go to
 * the neighbours and wrap round at the ends.
 */
export function rovingTarget(
  key: string,
  index: number,
  count: number,
  orientation: RovingOrientation,
): number | undefined {
  if (count <= 0 || index < 0 || index >= count) return undefined;
  const back = orientation === "horizontal" ? "ArrowLeft" : "ArrowUp";
  const forward = orientation === "horizontal" ? "ArrowRight" : "ArrowDown";
  switch (key) {
    case back:
      return (index - 1 + count) % count;
    case forward:
      return (index + 1) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return undefined;
  }
}

/**
 * Moves the keyboard as a key asks among the elements of `group` that match `members`,
 * from the one the event came from. Says whether it moved.
 */
export function roveWithin(
  event: { readonly key: string; readonly target: EventTarget | null },
  group: Element,
  members: string,
  orientation: RovingOrientation,
): boolean {
  const all = [...group.querySelectorAll<HTMLElement>(members)];
  const from = all.findIndex((member) => member === event.target);
  const to = rovingTarget(event.key, from, all.length, orientation);
  if (to === undefined || to === from) return false;
  all[to]?.focus();
  return true;
}
