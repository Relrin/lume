/**
 * Questions a component asks of the layout, each about one thing by id.
 *
 * Every selector returns an entry of the store as it is, or a value made of
 * primitives, so that a component which selects with one renders again only when its
 * own answer changes. The few that build a list say so, and are for callers outside
 * the component tree.
 */

import type { LayoutState, SplitEntry, TabEntry, TreeNode, WindowEntry } from "./store.ts";
import type { Pane, PaneId, SplitId, TabId, UnitRect, WindowId } from "./types.ts";

/** No ids: one list for every window that has none, so a selector's answer stays the same. */
const NO_IDS: readonly string[] = [];

/** What a tab is called when nothing names it. */
export const UNTITLED_TAB = "Terminal";

/** A window, or the page's own when none is named. */
export function selectWindow(state: LayoutState, window?: WindowId): WindowEntry | undefined {
  const id = window ?? state.window;
  return id === null ? undefined : state.windows[id];
}

/** The tabs of a window in order. The same array until the order changes. */
export function selectTabOrder(state: LayoutState, window?: WindowId): readonly TabId[] {
  return selectWindow(state, window)?.tabs ?? NO_IDS;
}

/** The tab a window has on show. */
export function selectActiveTab(state: LayoutState, window?: WindowId): TabId | null {
  return selectWindow(state, window)?.active ?? null;
}

/** The pane of a window that has the keyboard. */
export function selectFocus(state: LayoutState, window?: WindowId): PaneId | null {
  return selectWindow(state, window)?.focus ?? null;
}

/** The panes floating over a window, from the back to the front. */
export function selectFloating(state: LayoutState, window?: WindowId): readonly PaneId[] {
  return selectWindow(state, window)?.floating ?? NO_IDS;
}

/** One tab. */
export function selectTab(state: LayoutState, tab: TabId): TabEntry | undefined {
  return state.tabs[tab];
}

/** One pane, tiled or floating. */
export function selectPane(state: LayoutState, pane: PaneId): Pane | undefined {
  return state.panes[pane];
}

/** One split. */
export function selectSplit(state: LayoutState, split: SplitId): SplitEntry | undefined {
  return state.splits[split];
}

/** The shares of a split's children, in units. */
export function selectRatios(state: LayoutState, split: SplitId): readonly number[] | undefined {
  return state.splits[split]?.ratios;
}

/** The pane that fills a tab for now, hiding the others. */
export function selectZoomed(state: LayoutState, tab: TabId): PaneId | null {
  return state.tabs[tab]?.zoomed ?? null;
}

/** The tab a tiled pane is in. A floating pane is in none. */
export function selectTabOf(state: LayoutState, pane: PaneId): TabId | undefined {
  return state.paneTabs[pane];
}

/** Where a floating pane floats, in units of the area it floats over. */
export function selectFloatRect(state: LayoutState, pane: PaneId): UnitRect | undefined {
  return state.floats[pane]?.rect;
}

/** Whether a pane floats over a window. */
export function isFloating(state: LayoutState, pane: PaneId): boolean {
  return pane in state.floats;
}

/** Whether a pane has its window's keyboard. */
export function isFocused(state: LayoutState, pane: PaneId): boolean {
  const window = selectWindowOf(state, pane);
  return window !== undefined && state.windows[window]?.focus === pane;
}

/** The window a pane is shown in. */
export function selectWindowOf(state: LayoutState, pane: PaneId): WindowId | undefined {
  const tab = state.paneTabs[pane];
  if (tab !== undefined) return state.tabs[tab]?.window;
  return state.floats[pane]?.window;
}

/**
 * Whether a pane can be seen in its window: it floats, or it is in the tab on show and
 * no other pane of that tab is zoomed over it.
 */
export function isPaneVisible(state: LayoutState, pane: PaneId): boolean {
  if (pane in state.floats) return true;
  const tab = state.tabs[state.paneTabs[pane] ?? ""];
  if (!tab) return false;
  if (state.windows[tab.window]?.active !== tab.id) return false;
  return tab.zoomed === null || tab.zoomed === pane;
}

/** The ids of the panes under a node, in the order they are laid out. Builds a list. */
export function panesOf(node: TreeNode): PaneId[] {
  if (node.type === "pane") return [node.id];
  return node.children.flatMap(panesOf);
}

/** The panes of a tab, in the order they are laid out. Builds a list. */
export function selectTabPanes(state: LayoutState, tab: TabId): PaneId[] {
  const entry = state.tabs[tab];
  return entry ? panesOf(entry.root) : [];
}

/** The panes of the tab a window has on show, in the order they are laid out. Builds a list. */
export function selectActivePanes(state: LayoutState, window?: WindowId): PaneId[] {
  const active = selectActiveTab(state, window);
  return active === null ? [] : selectTabPanes(state, active);
}

/**
 * The panes of a window that can be seen: those of the tab on show, or the zoomed one
 * alone, then the floating ones from back to front. Builds a list.
 */
export function selectVisiblePanes(state: LayoutState, window?: WindowId): PaneId[] {
  const entry = selectWindow(state, window);
  if (!entry) return [];
  const tab = entry.active === null ? undefined : state.tabs[entry.active];
  const tiled = tab ? (tab.zoomed === null ? panesOf(tab.root) : [tab.zoomed]) : [];
  return [...tiled, ...entry.floating];
}

/** Every pane of a window: each tab's in order, then the floating ones. Builds a list. */
export function selectWindowPanes(state: LayoutState, window?: WindowId): PaneId[] {
  const entry = selectWindow(state, window);
  if (!entry) return [];
  return [...entry.tabs.flatMap((tab) => selectTabPanes(state, tab)), ...entry.floating];
}

/**
 * The pane a tab is named after: the one of its panes that last had the keyboard, or
 * its first.
 */
export function selectTitlePane(state: LayoutState, tab: TabId): PaneId | undefined {
  const entry = state.tabs[tab];
  if (!entry) return undefined;
  const last = state.lastFocus[tab];
  if (last !== undefined && state.paneTabs[last] === tab) return last;
  let node = entry.root;
  while (node.type === "split") {
    const first = node.children[0];
    if (!first) return undefined;
    node = first;
  }
  return node.id;
}

/**
 * What a tab is called: the name the user gave it, else the name the user gave the
 * pane it is named after, else what the program in that pane calls itself, else a
 * plain word. `paneTitles` holds the titles programs have set, by pane.
 */
export function tabTitle(
  state: LayoutState,
  tab: TabId,
  paneTitles: ReadonlyMap<PaneId, string>,
): string {
  const named = state.tabs[tab]?.title;
  if (named) return named;
  const pane = selectTitlePane(state, tab);
  if (pane === undefined) return UNTITLED_TAB;
  return state.panes[pane]?.title || paneTitles.get(pane) || UNTITLED_TAB;
}
