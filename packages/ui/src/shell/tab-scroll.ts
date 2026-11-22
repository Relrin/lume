/**
 * The row of tabs when it holds more than fits: it scrolls sideways, and this says how.
 * Which of its ends hide tabs, so that those ends fade; what a turn of the wheel moves
 * it by; how fast it moves under a tab that is dragged to one of its ends; and which
 * tabs are out of sight.
 *
 * Nothing here touches the page: the row measures itself and asks.
 */

/** How wide the fade at an end of the row is, while that end hides tabs. */
export const FADE_PX = 28;

/** How near an end of the row a dragged tab is taken to ask for more of it. */
export const EDGE_ZONE_PX = 48;

/** The most the row moves in one frame under a tab held at its end. */
export const EDGE_SPEED_MAX_PX = 14;

/** How much of a tab may lie past an end of the row before it counts as out of sight. */
export const AWAY_PX = 20;

/** How long after a key the row jumps to the tab on show, where it would have glided. */
export const KEY_RECENT_MS = 400;

/** One line of text, for a wheel that counts in lines. */
const LINE_PX = 16;

/** How far the row is scrolled, how much of it shows, and how long it is. */
export interface Extent {
  readonly scroll: number;
  readonly view: number;
  readonly full: number;
}

export interface Fades {
  /** Tabs are hidden past the row's start. */
  readonly start: boolean;
  /** Tabs are hidden past the row's end. */
  readonly end: boolean;
  /** The row holds more than fits. */
  readonly over: boolean;
}

/** Which ends of the row hide something. A pixel of rounding either way hides nothing. */
export function fades(extent: Extent): Fades {
  const most = Math.max(0, extent.full - extent.view);
  return {
    start: extent.scroll > 1,
    end: extent.scroll < most - 1,
    over: most > 1,
  };
}

/**
 * How far a turn of the wheel moves the row sideways, or null when the wheel is left
 * alone: a wheel that turns up and down scrolls the row, since it has no other way to;
 * one that already goes sideways, as two fingers on a trackpad do, does so by itself.
 * `mode` is the wheel's own unit: 0 pixels, 1 lines, 2 pages.
 */
export function wheelShift(dx: number, dy: number, mode: number, view: number): number | null {
  if (dy === 0 || Math.abs(dy) <= Math.abs(dx)) return null;
  const unit = mode === 1 ? LINE_PX : mode === 2 ? view : 1;
  return dy * unit;
}

/**
 * How far the row moves this frame under a pointer that drags a tab: nothing in its
 * middle, and towards an end faster the nearer the pointer is to it. Negative is
 * towards the start.
 */
export function edgeSpeed(pointer: number, start: number, end: number): number {
  if (end - start < EDGE_ZONE_PX * 2) return 0;
  const before = start + EDGE_ZONE_PX - pointer;
  if (before > 0)
    return -Math.ceil((Math.min(before, EDGE_ZONE_PX) / EDGE_ZONE_PX) * EDGE_SPEED_MAX_PX);
  const after = pointer - (end - EDGE_ZONE_PX);
  if (after > 0)
    return Math.ceil((Math.min(after, EDGE_ZONE_PX) / EDGE_ZONE_PX) * EDGE_SPEED_MAX_PX);
  return 0;
}

/** Where a tab lies along the row, in the same measure as the row's own ends. */
export interface Span {
  readonly id: string;
  readonly start: number;
  readonly end: number;
}

/** The tabs that are out of sight: all but a sliver of them lies past an end of the row. */
export function away(spans: readonly Span[], start: number, end: number): string[] {
  return spans
    .filter((span) => span.end - AWAY_PX < start || span.start + AWAY_PX > end)
    .map((span) => span.id);
}

/** Whether what a tab is called holds what was typed, whatever its letters' case. */
export function tabMatches(label: string, query: string): boolean {
  const wanted = query.trim().toLowerCase();
  return wanted === "" || label.toLowerCase().includes(wanted);
}
