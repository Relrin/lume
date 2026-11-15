/**
 * Shared by the layout tests: the recorded sequences of intents the core's own tests
 * check its layout against, a plain copy of a workspace that follows changes the
 * obvious way, and small trees to lay out.
 */

import { readFileSync } from "node:fs";

import type { LayoutView } from "../src/core/types.ts";
import type {
  Created,
  Intent,
  LayoutChange,
  Node,
  Pane,
  PaneId,
  Tab,
  Window,
  Workspace,
} from "../src/layout/types.ts";

const VECTORS = "../../../tests/layout/vectors/";

/** The names of the recorded sequences. */
export const VECTOR_NAMES = [
  "splits-and-tabs",
  "floating-and-scratch",
  "panels-and-windows",
] as const;

/** One step of a recorded sequence: an intent, and what it changed or why it was refused. */
export interface VectorStep {
  readonly intent: Intent;
  readonly changes?: readonly LayoutChange[];
  readonly created?: Created;
  readonly refused?: string;
}

/** A recorded sequence: where it starts, each step, and where it ends. */
export interface Vector {
  readonly name: string;
  readonly start: Workspace;
  readonly steps: readonly VectorStep[];
  readonly end: Workspace;
}

/** Reads one recorded sequence. */
export function vector(name: string): Vector {
  const json = readFileSync(new URL(`${VECTORS}${name}.json`, import.meta.url), "utf8");
  return JSON.parse(json) as Vector;
}

type Mutable<T> = { -readonly [Key in keyof T]: Mutable<T[Key]> };

function clone<T>(value: T): Mutable<T> {
  return structuredClone(value) as Mutable<T>;
}

/**
 * A workspace as a follower holds it: without what only the core keeps, which is the
 * history of the focus, where each tab's focus last was, and the next id.
 */
export function followerForm(workspace: Workspace): Workspace {
  const copy = clone(workspace);
  copy.next_id = 0;
  for (const window of copy.windows) {
    window.history = [];
    for (const tab of window.centre.tabs) tab.last_focus = null;
  }
  return copy;
}

function findPane(node: Mutable<Node>, id: PaneId): Mutable<Node> | undefined {
  if (node.type === "pane") return node.id === id ? node : undefined;
  for (const child of node.children) {
    const found = findPane(child, id);
    if (found) return found;
  }
  return undefined;
}

function findSplit(node: Mutable<Node>, id: string): Mutable<Node> | undefined {
  if (node.type === "pane") return undefined;
  if (node.id === id) return node;
  for (const child of node.children) {
    const found = findSplit(child, id);
    if (found) return found;
  }
  return undefined;
}

function viewOfTab(tab: Tab): Mutable<Tab> {
  return { ...clone(tab), last_focus: null };
}

function viewOfWindow(window: Window): Mutable<Window> {
  const copy = clone(window);
  copy.history = [];
  copy.centre.tabs = copy.centre.tabs.map(viewOfTab);
  return copy;
}

/**
 * A copy of a workspace that follows changes by editing one plain tree, as the core's
 * own reference follower does. The store is compared with it after every step.
 */
export class PlainFollower {
  readonly #workspace: Mutable<Workspace>;

  constructor(start: Workspace) {
    this.#workspace = clone(followerForm(start));
  }

  get workspace(): Workspace {
    return this.#workspace;
  }

  #tab(id: string): Mutable<Tab> | undefined {
    for (const window of this.#workspace.windows) {
      const tab = window.centre.tabs.find((other) => other.id === id);
      if (tab) return tab;
    }
    return undefined;
  }

  #setPane(pane: Pane): void {
    const put = (target: Mutable<Pane>) => {
      target.title = pane.title;
      target.content = clone(pane.content);
    };
    for (const window of this.#workspace.windows) {
      for (const tab of window.centre.tabs) {
        const found = findPane(tab.root, pane.id);
        if (found && found.type === "pane") return put(found);
      }
      const floating = window.floating.find((entry) => entry.pane.id === pane.id);
      if (floating) return put(floating.pane);
    }
    const parked = this.#workspace.scratch?.parked;
    if (parked && parked.pane.id === pane.id) put(parked.pane);
  }

  apply(change: LayoutChange): void {
    const workspace = this.#workspace;
    const window = (id: string) => workspace.windows.find((other) => other.id === id);
    switch (change.type) {
      case "window_opened":
        workspace.windows = workspace.windows.filter((other) => other.id !== change.window.id);
        workspace.windows.push(viewOfWindow(change.window));
        return;
      case "window_bounds": {
        const found = window(change.window);
        if (found) found.bounds = clone(change.bounds);
        return;
      }
      case "tab_added": {
        const tabs = window(change.window)?.centre.tabs;
        tabs?.splice(Math.min(change.index, tabs.length), 0, viewOfTab(change.tab));
        return;
      }
      case "tab_removed":
        for (const each of workspace.windows) {
          each.centre.tabs = each.centre.tabs.filter((tab) => tab.id !== change.tab);
        }
        return;
      case "tabs": {
        const found = window(change.window);
        if (!found) return;
        const place = (tab: Tab) => {
          const at = change.order.indexOf(tab.id);
          return at < 0 ? Number.MAX_SAFE_INTEGER : at;
        };
        found.centre.tabs.sort((a, b) => place(a) - place(b));
        found.centre.active = change.active;
        return;
      }
      case "tab_title": {
        const tab = this.#tab(change.tab);
        if (tab) tab.title = change.title;
        return;
      }
      case "tab_tree": {
        const tab = this.#tab(change.tab);
        if (!tab) return;
        tab.root = clone(change.root);
        tab.zoomed = change.zoomed;
        return;
      }
      case "ratios":
        for (const each of workspace.windows) {
          for (const tab of each.centre.tabs) {
            const split = findSplit(tab.root, change.split);
            if (split && split.type === "split") split.ratios = [...change.ratios];
          }
        }
        return;
      case "pane":
        this.#setPane(change.pane);
        return;
      case "floating": {
        const found = window(change.window);
        if (found) found.floating = clone(change.floating) as Mutable<Window>["floating"];
        return;
      }
      case "float_rect":
        for (const each of workspace.windows) {
          const floating = each.floating.find((entry) => entry.pane.id === change.pane);
          if (floating) floating.rect = { ...change.rect };
        }
        return;
      case "dock": {
        const found = window(change.window);
        if (found) found.dock[change.area] = clone(change.dock);
        return;
      }
      case "dock_size": {
        const found = window(change.window);
        if (found) found.dock[change.area].size = change.size;
        return;
      }
      case "focus": {
        const found = window(change.window);
        if (found) found.focus = change.pane;
        return;
      }
      case "window_closed":
        workspace.windows = workspace.windows.filter((other) => other.id !== change.window);
        return;
      case "scratch":
        workspace.scratch = clone(change.scratch);
        return;
      case "window_focused":
        workspace.focused_window = change.window;
        return;
      case "pane_closed":
        return;
    }
  }
}

/** A view of a workspace for the window `window`, at a place in the sequence of changes. */
export function viewOf(workspace: Workspace, window = "w1", epoch = 1, revision = 0): LayoutView {
  return { window, epoch, revision, workspace };
}

/** A terminal pane with nothing started in it. */
export function pane(id: PaneId, title: string | null = null): Node {
  return {
    type: "pane",
    id,
    title,
    content: {
      type: "terminal",
      backend: null,
      session: null,
      profile: null,
      launcher: null,
      cwd: null,
      env: [],
    },
  };
}

/** A split of some children. */
export function split(
  id: string,
  axis: "row" | "column",
  ratios: readonly number[],
  children: readonly Node[],
): Node {
  return { type: "split", id, axis, ratios, children };
}

/** A workspace with one window, `w1`, holding one tab, `t1`, with this tree. */
export function oneTab(root: Node, focus: PaneId | null = null): Workspace {
  const dock = { panels: [], size: 240, collapsed: false };
  return {
    id: "00000000000000a1",
    name: "test",
    windows: [
      {
        id: "w1",
        bounds: null,
        dock: { top: dock, left: dock, right: dock, bottom: dock },
        centre: {
          tabs: [{ id: "t1", title: null, root, zoomed: null, last_focus: focus }],
          active: "t1",
        },
        floating: [],
        focus,
        history: focus === null ? [] : [focus],
      },
    ],
    focused_window: "w1",
    scratch: null,
    next_id: 100,
  };
}
