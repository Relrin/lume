/**
 * Questions a component asks of the layout, each about one thing by id.
 *
 * Every selector returns an entry of the store as it is, or a value made of
 * primitives, so that a component which selects with one renders again only when its
 * own answer changes. The few that build a list say so, and are for callers outside
 * the component tree.
 */

import type { LayoutState, SplitEntry, TabEntry, TreeNode, WindowEntry } from "./store.ts";
import type {
  Area,
  DockArea,
  Group,
  GroupId,
  Pane,
  PaneId,
  Panel,
  PanelId,
  Shelved,
  SplitId,
  TabId,
  UnitRect,
  WindowId,
} from "./types.ts";

/** No ids: one list for every window that has none, so a selector's answer stays the same. */
const NO_IDS: readonly string[] = [];

/** No groups: one list for every window whose tabs are in none. */
const NO_GROUPS: readonly Group[] = [];

/** No panels: one list for every area that has none. */
const NO_PANELS: readonly Panel[] = [];

/** The id of the panel that lists a window's tabs, wherever it is docked. */
export const TAB_LIST_PANEL: PanelId = "lume.tabs";

/** The areas at a window's edges that panels can be moved between. The top bar is fixed. */
export const SIDE_AREAS = ["left", "right", "bottom"] as const satisfies readonly Area[];

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

/**
 * The workspace a window works in: the one of the tab it shows, or the one whose file
 * holds the layout.
 */
export function selectCurrentWorkspace(state: LayoutState, window?: WindowId): string {
  const tab = selectActiveTab(state, window);
  return (tab === null ? undefined : state.tabs[tab]?.workspace) ?? state.name;
}

/** The pane of a window that has the keyboard. */
export function selectFocus(state: LayoutState, window?: WindowId): PaneId | null {
  return selectWindow(state, window)?.focus ?? null;
}

/** The panes floating over a window, from the back to the front. */
export function selectFloating(state: LayoutState, window?: WindowId): readonly PaneId[] {
  return selectWindow(state, window)?.floating ?? NO_IDS;
}

/**
 * Those of a window's floating panes that are put away at its corner, in the order
 * they float in. The same array until it changes.
 */
export function selectMinimised(state: LayoutState, window?: WindowId): readonly PaneId[] {
  return selectWindow(state, window)?.minimised ?? NO_IDS;
}

/** Whether a floating pane is put away at the corner of its window. */
export function isMinimised(state: LayoutState, pane: PaneId): boolean {
  return (state.floats[pane]?.minimised ?? null) !== null;
}

/** One of the areas at a window's edges. The same object until that area changes. */
export function selectDock(
  state: LayoutState,
  area: Area,
  window?: WindowId,
): DockArea | undefined {
  return selectWindow(state, window)?.dock[area];
}

/** The panels docked at one edge of a window, in order. */
export function selectPanels(state: LayoutState, area: Area, window?: WindowId): readonly Panel[] {
  return selectDock(state, area, window)?.panels ?? NO_PANELS;
}

/** The area a panel is docked in, if the window has the panel at one of its sides. */
export function selectPanelArea(
  state: LayoutState,
  panel: PanelId,
  window?: WindowId,
): (typeof SIDE_AREAS)[number] | null {
  const dock = selectWindow(state, window)?.dock;
  if (!dock) return null;
  return SIDE_AREAS.find((area) => dock[area].panels.some((held) => held.id === panel)) ?? null;
}

/**
 * Whether a window's tabs are listed in a panel on show at one of its sides. When they
 * are, the top bar does not list them as well. A side that is folded away shows
 * nothing, so its list does not count: the tabs are always somewhere to be seen.
 */
export function hasTabListPanel(state: LayoutState, window?: WindowId): boolean {
  const area = selectPanelArea(state, TAB_LIST_PANEL, window);
  return area !== null && selectDock(state, area, window)?.collapsed === false;
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
  const float = state.floats[pane];
  if (float) return float.minimised === null;
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
  return [...tiled, ...entry.floating.filter((pane) => !isMinimised(state, pane))];
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

/** The groups of a window's tabs, in the order their first tabs come. */
export function selectGroups(state: LayoutState, window?: WindowId): readonly Group[] {
  return selectWindow(state, window)?.groups ?? NO_GROUPS;
}

/** One group of a window's tabs. */
export function selectGroup(
  state: LayoutState,
  group: GroupId,
  window?: WindowId,
): Group | undefined {
  return selectGroups(state, window).find((found) => found.id === group);
}

/**
 * Whether a tab is muted: the tab itself, or the group it is in. What runs in a muted
 * tab does not ask for attention.
 */
export function isTabMuted(state: LayoutState, tab: TabId): boolean {
  const entry = state.tabs[tab];
  if (!entry) return false;
  if (entry.muted) return true;
  if (entry.group === null) return false;
  return selectGroup(state, entry.group, entry.window)?.muted === true;
}

/** The tabs of a group, in order. Builds a list. */
export function selectGroupTabs(state: LayoutState, group: GroupId, window?: WindowId): TabId[] {
  return selectTabOrder(state, window).filter((tab) => state.tabs[tab]?.group === group);
}

/** The kept groups that were closed, the most recently closed last. */
export function selectShelf(state: LayoutState): readonly Shelved[] {
  return state.shelf;
}

/** One thing along a window's row of tabs: the label of a group, or a tab. */
export type StripItem =
  | {
      readonly kind: "group";
      readonly group: Group;
      /** The group's tabs, in order. */
      readonly tabs: readonly TabId[];
    }
  | {
      readonly kind: "tab";
      readonly tab: TabId;
      /** The tab's place among the window's tabs. */
      readonly index: number;
      /** The group the tab is in. */
      readonly group: Group | null;
      /** Whether its group is folded, which hides it. */
      readonly folded: boolean;
      /** Whether it is the last tab of its group: its group's line ends with it. */
      readonly last: boolean;
    };

/**
 * A window's row of tabs as it is laid out: each group's label in front of its first
 * tab, then its tabs. Builds a list; a component selects {@link selectStripKey} and
 * builds this when that changes.
 */
export function stripItems(state: LayoutState, window?: WindowId): StripItem[] {
  const order = selectTabOrder(state, window);
  const groups = selectGroups(state, window);
  const items: StripItem[] = [];
  order.forEach((tab, index) => {
    const id = state.tabs[tab]?.group ?? null;
    const group = id === null ? null : (groups.find((found) => found.id === id) ?? null);
    const before = index > 0 ? state.tabs[order[index - 1] ?? ""]?.group : undefined;
    const after = state.tabs[order[index + 1] ?? ""]?.group;
    if (group && before !== group.id) {
      items.push({
        kind: "group",
        group,
        tabs: order.filter((other) => state.tabs[other]?.group === group.id),
      });
    }
    items.push({
      kind: "tab",
      tab,
      index,
      group,
      folded: group?.collapsed === true,
      last: group !== null && after !== group.id,
    });
  });
  return items;
}

/**
 * Says what {@link stripItems} would build, as text: the same text until the row of
 * tabs changes.
 */
export function selectStripKey(state: LayoutState, window?: WindowId): string {
  const order = selectTabOrder(state, window);
  const groups = selectGroups(state, window);
  const tabs = order.map((tab) => `${tab}:${state.tabs[tab]?.group ?? ""}`).join(" ");
  return groups.length === 0 ? tabs : `${tabs}|${JSON.stringify(groups)}`;
}
