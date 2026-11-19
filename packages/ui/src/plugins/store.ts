/**
 * What the plugins add to the window, and what their code shows in it now.
 *
 * The core works everything out from the plugins' manifests: the menus with their
 * plugins' items in place, the status items of each bar, the panels, the pictures,
 * the motion and the cursor effects. The page fetches that whole when it starts and
 * whenever it is told it changed. What the plugins' code shows (a status item's value,
 * a panel's rows, a condition it set) arrives in notices as it changes, and is kept
 * here beside it. All a plugin showed in one turn of its code comes as one notice and
 * is taken in in one step, so the window draws it together and never the half of it.
 *
 * A panel's rows come only while the panel is on screen in this window, and whole
 * again each time it comes on screen; the rows of one that went are kept until then.
 *
 * Nothing here touches the page, so all of it is tested with plain objects.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type {
  IconRef,
  MenuNodeData,
  Notice,
  PanelRowData,
  PluginImage,
  UiContributions,
} from "../core/types.ts";

/** What a status item of a plugin's shows now: the fields its code last set. */
export type StatusValues = Readonly<Record<string, unknown>>;

/** What the plugins add, and what their code shows. */
export interface PluginsState {
  /** What the core last said the plugins add; nothing before it has said. */
  readonly shown: UiContributions | null;
  /** What each status item shows, by `<item>` or `<item>@<pane>`. */
  readonly values: ReadonlyMap<string, StatusValues>;
  /** What each list panel shows, by panel. */
  readonly rows: ReadonlyMap<string, readonly PanelRowData[]>;
  /** The conditions plugins set that hold now. */
  readonly contexts: ReadonlySet<string>;
  /** What went wrong with each plugin's code, by plugin, where something did. */
  readonly problems: ReadonlyMap<string, string>;
  /** Takes in what the core sent whole. */
  set(shown: UiContributions): void;
  /** Follows a notice about plugins. Says whether it was one. */
  follow(notice: Notice): boolean;
}

export type PluginsStore = StoreApi<PluginsState>;

/** Where a value is kept: an item's own, or its value for one pane. */
export function valueKey(item: string, pane: string | null | undefined): string {
  return pane === null || pane === undefined ? item : `${item}@${pane}`;
}

/** Makes the store. It holds nothing until the core says what the plugins add. */
export function createPluginsStore(): PluginsStore {
  return createStore<PluginsState>()((set, get) => ({
    shown: null,
    values: new Map(),
    rows: new Map(),
    contexts: new Set(),
    problems: new Map(),
    set: (shown) => {
      const values = new Map<string, StatusValues>();
      for (const entry of shown.values) values.set(valueKey(entry.item, entry.pane), entry.value);
      // The rows of panels there still are stay until the core sends them again.
      const panels = new Set(shown.panels.map((panel) => panel.id));
      const rows = new Map<string, readonly PanelRowData[]>(
        [...get().rows].filter(([panel]) => panels.has(panel)),
      );
      for (const [panel, given] of Object.entries(shown.rows)) rows.set(panel, given);
      const contexts = new Set(
        Object.entries(shown.contexts)
          .filter(([, on]) => on)
          .map(([name]) => name),
      );
      const problems = new Map<string, string>();
      for (const plugin of shown.plugins) {
        if (plugin.problem !== null) problems.set(plugin.id, plugin.problem);
      }
      set({ shown, values, rows, contexts, problems });
    },
    follow: (notice) => {
      switch (notice.type) {
        case "plugin_frame": {
          const now = get();
          const next: {
            values?: PluginsState["values"];
            rows?: PluginsState["rows"];
            contexts?: PluginsState["contexts"];
          } = {};
          if (notice.statuses.length > 0) {
            const values = new Map(now.values);
            for (const status of notice.statuses) {
              const key = valueKey(status.item, status.pane);
              if (status.value === null) values.delete(key);
              else values.set(key, status.value);
            }
            next.values = values;
          }
          const panels = Object.entries(notice.panels);
          if (panels.length > 0) {
            const rows = new Map(now.rows);
            for (const [panel, given] of panels) {
              if (given === null) rows.delete(panel);
              else rows.set(panel, given);
            }
            next.rows = rows;
          }
          const conditions = Object.entries(notice.contexts);
          if (conditions.length > 0) {
            const contexts = new Set(now.contexts);
            for (const [name, on] of conditions) {
              if (on) contexts.add(name);
              else contexts.delete(name);
            }
            next.contexts = contexts;
          }
          // One step: everything the plugin's turn showed is drawn together.
          set(next);
          return true;
        }
        case "plugin_state": {
          const problems = new Map(get().problems);
          if (notice.problem === null) problems.delete(notice.plugin);
          else problems.set(notice.plugin, notice.problem);
          set({ problems });
          return true;
        }
        default:
          return false;
      }
    },
  }));
}

/**
 * Whether conditions hold: each a context's name, perhaps after `!`. A context of a
 * plugin's holds while the plugin says so; the others are given.
 */
export function holds(when: readonly string[] | undefined, contexts: ReadonlySet<string>): boolean {
  if (when === undefined) return true;
  return when.every((term) =>
    term.startsWith("!") ? !contexts.has(term.slice(1)) : contexts.has(term),
  );
}

/**
 * The picture a plugin's code names: one of the plugin's own, by the name its manifest
 * gives it, or else one of the page's by name.
 */
export function iconNamed(
  plugin: string,
  name: string | undefined,
  images: Readonly<Record<string, PluginImage>> | undefined,
): IconRef | undefined {
  if (name === undefined || name === "") return undefined;
  const own = `${plugin}:${name}`;
  if (images?.[own] !== undefined) return { image: own };
  return { glyph: name };
}

/** The node of a menu with a key, at any depth. */
export function findNode(
  nodes: readonly MenuNodeData[] | undefined,
  key: string,
): MenuNodeData | undefined {
  for (const node of nodes ?? []) {
    if (node.key === key) return node;
    const inside = findNode(node.children, key);
    if (inside) return inside;
  }
  return undefined;
}
