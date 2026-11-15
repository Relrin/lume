/**
 * The page's copy of the layout.
 *
 * The core owns the layout and numbers its changes. A page takes the layout whole
 * once, then follows the changes in order: a change is applied only when it is the
 * next one, and anything else is either old news or a gap that a fresh view mends.
 *
 * The copy is kept by id rather than as one tree. A component that shows one pane
 * reads that pane's entry, and a change to another pane, or to a split's ratios,
 * leaves the entry untouched and the component alone: an entry is replaced only when
 * what it holds really changed.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { LayoutView } from "../core/types.ts";
import type {
  Area,
  Axis,
  Bounds,
  DockAreas,
  Floating,
  LayoutChange,
  Node,
  Pane,
  PaneId,
  Scratch,
  SplitId,
  Tab,
  TabId,
  UnitRect,
  Window,
  WindowId,
  Workspace,
} from "./types.ts";

/** A tab's tree with only its shape: what each node shows is kept by id. */
export type TreeNode =
  | { readonly type: "pane"; readonly id: PaneId }
  | {
      readonly type: "split";
      readonly id: SplitId;
      readonly axis: Axis;
      readonly children: readonly TreeNode[];
    };

/** One window, without its tabs' and panes' contents. */
export interface WindowEntry {
  readonly id: WindowId;
  readonly bounds: Bounds | null;
  readonly dock: DockAreas;
  /** The window's tabs in order. */
  readonly tabs: readonly TabId[];
  /** The tab on show. */
  readonly active: TabId | null;
  /** The floating panes, from the one at the back to the one in front. */
  readonly floating: readonly PaneId[];
  /** The pane that has the keyboard. */
  readonly focus: PaneId | null;
}

/** One tab, without its panes' contents and its splits' ratios. */
export interface TabEntry {
  readonly id: TabId;
  readonly window: WindowId;
  /** The name the user gave the tab. */
  readonly title: string | null;
  readonly root: TreeNode;
  /** The pane that fills the tab for now, hiding the others. */
  readonly zoomed: PaneId | null;
}

/** One split's direction and its children's shares. */
export interface SplitEntry {
  readonly id: SplitId;
  readonly tab: TabId;
  readonly axis: Axis;
  /** Each child's share, in units. */
  readonly ratios: readonly number[];
}

/** Where a floating pane floats. */
export interface FloatEntry {
  readonly window: WindowId;
  readonly rect: UnitRect;
}

/** What following a set of changes came to. */
export type Followed =
  /** The changes were the next ones, and the copy has them. */
  | "applied"
  /** The changes are already in the copy, or belong to a layout it has left behind. */
  | "stale"
  /** Changes are missing between the copy and these. Nothing was changed. */
  | "gap";

/** The layout by id, and where it stands in the sequence of its changes. */
export interface LayoutState {
  /** Whether a layout has been taken yet. */
  readonly ready: boolean;
  /** The window this page is. */
  readonly window: WindowId | null;
  readonly epoch: number;
  readonly revision: number;
  /** The workspace's id and name. */
  readonly workspaceId: string;
  readonly name: string;
  /** The window the user last worked in. */
  readonly focusedWindow: WindowId | null;
  readonly scratch: Scratch | null;
  /** The workspace's windows in order. */
  readonly windowOrder: readonly WindowId[];
  readonly windows: Readonly<Record<WindowId, WindowEntry>>;
  readonly tabs: Readonly<Record<TabId, TabEntry>>;
  /** Every pane of a tab and every floating pane. */
  readonly panes: Readonly<Record<PaneId, Pane>>;
  readonly splits: Readonly<Record<SplitId, SplitEntry>>;
  /** The tab each tiled pane is in. */
  readonly paneTabs: Readonly<Record<PaneId, TabId>>;
  readonly floats: Readonly<Record<PaneId, FloatEntry>>;
  /** The pane of each tab that last had the keyboard, as far as this page saw. */
  readonly lastFocus: Readonly<Record<TabId, PaneId>>;

  /** Replaces the copy with a layout taken whole. */
  reset(view: LayoutView): void;
  /**
   * Follows the changes that took the layout to `revision` of `epoch`. They are
   * applied only when that is the revision after the copy's own.
   */
  apply(epoch: number, revision: number, changes: readonly LayoutChange[]): Followed;
}

/** The store a page keeps its layout in. */
export type LayoutStore = StoreApi<LayoutState>;

/** The parts of the state that hold the layout itself. */
type Data = Omit<LayoutState, "reset" | "apply">;

const EMPTY: Data = {
  ready: false,
  window: null,
  epoch: 0,
  revision: 0,
  workspaceId: "",
  name: "",
  focusedWindow: null,
  scratch: null,
  windowOrder: [],
  windows: {},
  tabs: {},
  panes: {},
  splits: {},
  paneTabs: {},
  floats: {},
  lastFocus: {},
};

/** The records of a state, each copied the first time it is written. */
class Draft {
  readonly #next: { -readonly [Key in keyof Data]: Data[Key] };
  readonly #owned = new Set<RecordKey>();

  constructor(base: Data) {
    this.#next = { ...base };
  }

  get data(): Data {
    return this.#next;
  }

  /** A record that may be written to: a copy, made once. */
  #own<Key extends RecordKey>(key: Key): Record<string, Entry<Key>> {
    let record = this.#next[key] as Record<string, Entry<Key>>;
    if (!this.#owned.has(key)) {
      this.#owned.add(key);
      record = { ...record };
      this.put(key, record as Data[Key]);
    }
    return record;
  }

  /** Puts an entry in a record, unless the record holds that very entry already. */
  set<Key extends RecordKey>(key: Key, id: string, entry: Entry<Key>): void {
    if ((this.#next[key] as Record<string, Entry<Key>>)[id] === entry) return;
    this.#own(key)[id] = entry;
  }

  remove(key: RecordKey, id: string): void {
    if (!(id in this.#next[key])) return;
    delete this.#own(key)[id];
  }

  put<Key extends keyof Data>(key: Key, value: Data[Key]): void {
    this.#next[key] = value;
  }
}

type RecordKey = "windows" | "tabs" | "panes" | "splits" | "paneTabs" | "floats" | "lastFocus";
type Entry<Key extends RecordKey> = Data[Key][string];

/** Whether two values are written the same way as JSON. */
function alike(a: unknown, b: unknown): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b);
}

function sameNumbers(a: readonly number[], b: readonly number[]): boolean {
  return a.length === b.length && a.every((value, index) => value === b[index]);
}

/** The ids of the panes under a node, in the order they are laid out. */
function paneIds(node: TreeNode, found: PaneId[] = []): PaneId[] {
  if (node.type === "pane") found.push(node.id);
  else for (const child of node.children) paneIds(child, found);
  return found;
}

function splitIds(node: TreeNode, found: SplitId[] = []): SplitId[] {
  if (node.type === "split") {
    found.push(node.id);
    for (const child of node.children) splitIds(child, found);
  }
  return found;
}

function sameShape(a: TreeNode, b: TreeNode): boolean {
  if (a.type === "pane" || b.type === "pane") return a.type === b.type && a.id === b.id;
  return (
    a.id === b.id &&
    a.axis === b.axis &&
    a.children.length === b.children.length &&
    a.children.every((child, index) => sameShape(child, b.children[index]!))
  );
}

/**
 * Takes a tab's tree into the draft: the panes and splits by id, and the tree's shape
 * as the return value. Entries that hold what they held before are kept as they are.
 */
function takeTree(draft: Draft, tab: TabId, node: Node): TreeNode {
  if (node.type === "pane") {
    const pane: Pane = { id: node.id, title: node.title, content: node.content };
    const before = draft.data.panes[pane.id];
    draft.set("panes", pane.id, before && alike(before, pane) ? before : pane);
    draft.set("paneTabs", pane.id, tab);
    draft.remove("floats", pane.id);
    return { type: "pane", id: pane.id };
  }
  const before = draft.data.splits[node.id];
  const same =
    before !== undefined &&
    before.tab === tab &&
    before.axis === node.axis &&
    sameNumbers(before.ratios, node.ratios);
  if (!same) {
    draft.set("splits", node.id, { id: node.id, tab, axis: node.axis, ratios: node.ratios });
  }
  return {
    type: "split",
    id: node.id,
    axis: node.axis,
    children: node.children.map((child) => takeTree(draft, tab, child)),
  };
}

/** Lets go of what a tab's old tree held and its new one does not. */
function dropTree(draft: Draft, tab: TabId, old: TreeNode, kept: TreeNode | undefined): void {
  const panes = new Set(kept ? paneIds(kept) : []);
  const splits = new Set(kept ? splitIds(kept) : []);
  for (const pane of paneIds(old)) {
    // A pane that has since been taken into another tab, or floats, is no longer this
    // tab's to let go of.
    if (panes.has(pane) || draft.data.paneTabs[pane] !== tab) continue;
    draft.remove("paneTabs", pane);
    if (!(pane in draft.data.floats)) draft.remove("panes", pane);
  }
  for (const split of splitIds(old)) {
    if (!splits.has(split) && draft.data.splits[split]?.tab === tab) draft.remove("splits", split);
  }
}

/** Puts a tab's tree in place of the one it had, if it had one. */
function setTree(
  draft: Draft,
  window: WindowId,
  tab: Pick<Tab, "id" | "title" | "root" | "zoomed">,
): void {
  const before = draft.data.tabs[tab.id];
  const taken = takeTree(draft, tab.id, tab.root);
  if (before) dropTree(draft, tab.id, before.root, taken);
  const root = before && sameShape(before.root, taken) ? before.root : taken;
  const same =
    before !== undefined &&
    before.window === window &&
    before.title === tab.title &&
    before.zoomed === tab.zoomed &&
    before.root === root;
  if (!same)
    draft.set("tabs", tab.id, { id: tab.id, window, title: tab.title, root, zoomed: tab.zoomed });
}

function takeTab(draft: Draft, window: WindowId, tab: Tab): void {
  setTree(draft, window, tab);
  if (tab.last_focus !== null) draft.set("lastFocus", tab.id, tab.last_focus);
}

function dropTab(draft: Draft, tab: TabId): void {
  const entry = draft.data.tabs[tab];
  if (!entry) return;
  dropTree(draft, tab, entry.root, undefined);
  draft.remove("tabs", tab);
  draft.remove("lastFocus", tab);
}

/** Puts a window's floating panes in place of those it had. */
function setFloating(draft: Draft, window: WindowId, floating: readonly Floating[]): PaneId[] {
  const now = new Set(floating.map((entry) => entry.pane.id));
  const before = draft.data.windows[window]?.floating ?? [];
  for (const pane of before) {
    // One that floats over another window by now is that window's.
    if (now.has(pane) || draft.data.floats[pane]?.window !== window) continue;
    draft.remove("floats", pane);
    if (!(pane in draft.data.paneTabs)) draft.remove("panes", pane);
  }
  for (const { pane, rect } of floating) {
    const held = draft.data.panes[pane.id];
    draft.set("panes", pane.id, held && alike(held, pane) ? held : pane);
    draft.remove("paneTabs", pane.id);
    const float = draft.data.floats[pane.id];
    const same = float !== undefined && float.window === window && alike(float.rect, rect);
    if (!same) draft.set("floats", pane.id, { window, rect });
  }
  return floating.map((entry) => entry.pane.id);
}

function takeWindow(draft: Draft, window: Window): void {
  dropWindow(draft, window.id);
  for (const tab of window.centre.tabs) takeTab(draft, window.id, tab);
  const floating = setFloating(draft, window.id, window.floating);
  draft.set("windows", window.id, {
    id: window.id,
    bounds: window.bounds,
    dock: window.dock,
    tabs: window.centre.tabs.map((tab) => tab.id),
    active: window.centre.active,
    floating,
    focus: window.focus,
  });
  draft.put("windowOrder", [...draft.data.windowOrder, window.id]);
}

function dropWindow(draft: Draft, window: WindowId): void {
  const entry = draft.data.windows[window];
  if (!entry) return;
  for (const tab of entry.tabs) dropTab(draft, tab);
  setFloating(draft, window, []);
  draft.remove("windows", window);
  draft.put(
    "windowOrder",
    draft.data.windowOrder.filter((other) => other !== window),
  );
}

/** Changes one window's entry, if the page has the window. */
function editWindow(
  draft: Draft,
  window: WindowId,
  edit: (entry: WindowEntry) => WindowEntry,
): void {
  const entry = draft.data.windows[window];
  if (entry) draft.set("windows", window, edit(entry));
}

function dockWith(dock: DockAreas, area: Area, edit: (area: DockAreas[Area]) => DockAreas[Area]) {
  return { ...dock, [area]: edit(dock[area]) };
}

/**
 * Brings the draft up to date with one change. A change about something the copy does
 * not have is passed over: a page that has missed changes notices by their numbering.
 */
function follow(draft: Draft, change: LayoutChange): void {
  switch (change.type) {
    case "window_opened":
      takeWindow(draft, change.window);
      return;
    case "window_bounds":
      editWindow(draft, change.window, (entry) => ({ ...entry, bounds: change.bounds }));
      return;
    case "tab_added":
      if (!draft.data.windows[change.window]) return;
      takeTab(draft, change.window, change.tab);
      editWindow(draft, change.window, (entry) => {
        const tabs = entry.tabs.filter((tab) => tab !== change.tab.id);
        tabs.splice(Math.min(change.index, tabs.length), 0, change.tab.id);
        return { ...entry, tabs };
      });
      return;
    case "tab_removed":
      for (const window of draft.data.windowOrder) {
        editWindow(draft, window, (entry) =>
          entry.tabs.includes(change.tab)
            ? { ...entry, tabs: entry.tabs.filter((tab) => tab !== change.tab) }
            : entry,
        );
      }
      dropTab(draft, change.tab);
      return;
    case "tabs":
      editWindow(draft, change.window, (entry) => {
        const place = (tab: TabId) => {
          const at = change.order.indexOf(tab);
          return at < 0 ? Number.MAX_SAFE_INTEGER : at;
        };
        // Sorting keeps the order of tabs the change does not name, as the core's does.
        const tabs = [...entry.tabs].sort((a, b) => place(a) - place(b));
        const moved = tabs.some((tab, index) => tab !== entry.tabs[index]);
        if (!moved && entry.active === change.active) return entry;
        return { ...entry, tabs: moved ? tabs : entry.tabs, active: change.active };
      });
      return;
    case "tab_title": {
      const tab = draft.data.tabs[change.tab];
      if (tab) draft.set("tabs", tab.id, { ...tab, title: change.title });
      return;
    }
    case "tab_tree": {
      const tab = draft.data.tabs[change.tab];
      if (!tab) return;
      setTree(draft, tab.window, {
        id: tab.id,
        title: tab.title,
        root: change.root,
        zoomed: change.zoomed,
      });
      return;
    }
    case "ratios": {
      const split = draft.data.splits[change.split];
      if (split) draft.set("splits", split.id, { ...split, ratios: change.ratios });
      return;
    }
    case "pane": {
      const { pane } = change;
      if (draft.data.panes[pane.id]) {
        draft.set("panes", pane.id, pane);
        return;
      }
      // The scratch terminal is a pane too while it is put away.
      const scratch = draft.data.scratch;
      if (scratch?.parked?.pane.id === pane.id) {
        draft.put("scratch", { ...scratch, parked: { ...scratch.parked, pane } });
      }
      return;
    }
    case "floating": {
      if (!draft.data.windows[change.window]) return;
      const floating = setFloating(draft, change.window, change.floating);
      editWindow(draft, change.window, (entry) => ({ ...entry, floating }));
      return;
    }
    case "float_rect": {
      const float = draft.data.floats[change.pane];
      if (float) draft.set("floats", change.pane, { ...float, rect: change.rect });
      return;
    }
    case "dock":
      editWindow(draft, change.window, (entry) => ({
        ...entry,
        dock: dockWith(entry.dock, change.area, () => change.dock),
      }));
      return;
    case "dock_size":
      editWindow(draft, change.window, (entry) => ({
        ...entry,
        dock: dockWith(entry.dock, change.area, (area) => ({ ...area, size: change.size })),
      }));
      return;
    case "focus": {
      editWindow(draft, change.window, (entry) => ({ ...entry, focus: change.pane }));
      const tab = change.pane === null ? undefined : draft.data.paneTabs[change.pane];
      if (tab !== undefined && change.pane !== null) draft.set("lastFocus", tab, change.pane);
      return;
    }
    case "window_closed":
      dropWindow(draft, change.window);
      return;
    case "scratch":
      draft.put("scratch", change.scratch);
      return;
    case "window_focused":
      draft.put("focusedWindow", change.window);
      return;
    case "pane_closed":
      // The changes before it have already taken the pane out.
      return;
  }
}

function dataOf(view: LayoutView): Data {
  const draft = new Draft(EMPTY);
  const { workspace } = view;
  for (const window of workspace.windows) takeWindow(draft, window);
  draft.put("ready", true);
  draft.put("window", view.window);
  draft.put("epoch", view.epoch);
  draft.put("revision", view.revision);
  draft.put("workspaceId", workspace.id);
  draft.put("name", workspace.name);
  draft.put("focusedWindow", workspace.focused_window);
  draft.put("scratch", workspace.scratch);
  return draft.data;
}

/** Makes a page's layout store. It holds nothing until it is given a layout. */
export function createLayoutStore(): LayoutStore {
  return createStore<LayoutState>()((set, get) => ({
    ...EMPTY,
    reset: (view) => set(dataOf(view)),
    apply: (epoch, revision, changes) => {
      const state = get();
      if (!state.ready) return "gap";
      if (epoch !== state.epoch) return epoch < state.epoch ? "stale" : "gap";
      if (revision <= state.revision) return "stale";
      if (revision !== state.revision + 1) return "gap";
      const draft = new Draft(state);
      for (const change of changes) follow(draft, change);
      draft.put("revision", revision);
      set(draft.data);
      return "applied";
    },
  }));
}

/** A tab's tree as the core writes it, put back together from the entries by id. */
export function nodeOf(state: LayoutState, node: TreeNode): Node {
  if (node.type === "pane") {
    const pane = state.panes[node.id];
    if (!pane) throw new Error(`the layout has no pane ${node.id}`);
    return { type: "pane", ...pane };
  }
  return {
    type: "split",
    id: node.id,
    axis: node.axis,
    ratios: state.splits[node.id]?.ratios ?? [],
    children: node.children.map((child) => nodeOf(state, child)),
  };
}

/**
 * The workspace the copy holds, as the core writes one. What only the core keeps is
 * left empty: the history of the focus, where each tab's focus last was, and the next
 * id. A copy that has followed every change equals the core's workspace in every
 * other respect.
 */
export function toWorkspace(state: LayoutState): Workspace {
  const windows = state.windowOrder.flatMap((id): Window[] => {
    const window = state.windows[id];
    if (!window) return [];
    const tabs = window.tabs.flatMap((tabId): Tab[] => {
      const tab = state.tabs[tabId];
      if (!tab) return [];
      return [
        {
          id: tab.id,
          title: tab.title,
          root: nodeOf(state, tab.root),
          zoomed: tab.zoomed,
          last_focus: null,
        },
      ];
    });
    const floating = window.floating.flatMap((paneId): Floating[] => {
      const pane = state.panes[paneId];
      const float = state.floats[paneId];
      return pane && float ? [{ pane, rect: float.rect }] : [];
    });
    return [
      {
        id: window.id,
        bounds: window.bounds,
        dock: window.dock,
        centre: { tabs, active: window.active },
        floating,
        focus: window.focus,
        history: [],
      },
    ];
  });
  return {
    id: state.workspaceId,
    name: state.name,
    windows,
    focused_window: state.focusedWindow,
    scratch: state.scratch,
    next_id: 0,
  };
}
