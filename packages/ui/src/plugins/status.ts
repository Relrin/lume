/**
 * The plugins' status items, as the window's bars draw them.
 *
 * A plugin's manifest says what each of its items is and where it goes; its code says,
 * as it runs, what the item shows. An item in a bar is drawn from its manifest until
 * its code says more, and only while the switch of its plugin's settings that it is
 * shown under, if it names one, is on. It is in the place its manifest gives, or in
 * the one the plugin's setting that chooses it holds. An item in a pane's header is
 * drawn only in the headers of the panes its code shows it for. An item whose conditions do not hold, or that its
 * code hides, is not drawn. An item of a plugin whose code was stopped for going over
 * its limits says so.
 *
 * The items go into the window's status registry beside any other, under their own
 * ids, and are taken out of it when they go.
 */

import type { ConfigView, StatusDef } from "../core/types.ts";
import type { ConfigStore } from "../config/store.ts";
import type { StatusStore } from "../status/registry.ts";
import type { StatusCardRow, StatusItem, StatusSlot, StatusTone } from "../status/types.ts";
import {
  holds,
  iconNamed,
  type PluginsState,
  type PluginsStore,
  type StatusValues,
} from "./store.ts";

const TONES: readonly StatusTone[] = ["neutral", "info", "success", "warning", "danger"];
const SLOTS: readonly StatusSlot[] = [
  "topbar.left",
  "topbar.center",
  "topbar.right",
  "statusbar.left",
  "statusbar.right",
  "pane.header",
];

function text(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function number(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function numbers(value: unknown): number[] | undefined {
  return Array.isArray(value)
    ? value.filter((point): point is number => typeof point === "number")
    : undefined;
}

function tone(value: unknown): StatusTone | undefined {
  return TONES.includes(value as StatusTone) ? (value as StatusTone) : undefined;
}

function card(value: unknown): StatusCardRow[] | undefined {
  if (!Array.isArray(value)) return undefined;
  return value.flatMap((row: unknown) => {
    if (typeof row !== "object" || row === null) return [];
    const { label, value: shown, points, tone: rowTone } = row as Record<string, unknown>;
    if (typeof label !== "string" || typeof shown !== "string") return [];
    const line = numbers(points);
    const coloured = tone(rowTone);
    return [
      {
        label,
        value: shown,
        ...(line ? { points: line } : {}),
        ...(coloured ? { tone: coloured } : {}),
      },
    ];
  });
}

/** One item as it is drawn, from its manifest and what its code last said. */
function itemOf(
  def: StatusDef,
  id: string,
  values: StatusValues | undefined,
  pane: string | undefined,
  state: PluginsState,
  view: ConfigView | null,
): StatusItem | undefined {
  const value = values ?? {};
  if (value["hidden"] === true) return undefined;
  const slot = slotOf(def, view);
  if (slot === undefined) return undefined;
  const label = text(value["label"]) ?? def.label ?? "";
  const shownText = text(value["text"]);
  const picture = iconNamed(def.plugin, text(value["icon"]), state.shown?.images) ?? def.icon;
  const problem = state.problems.get(def.plugin);
  const rows = card(value["card"]);
  const base = {
    id,
    slot,
    order: def.order,
    plugin: def.plugin,
    ...(tone(value["tone"]) ? { tone: tone(value["tone"]) } : {}),
    ...((text(value["tooltip"]) ?? def.tooltip)
      ? { tooltip: text(value["tooltip"]) ?? def.tooltip }
      : {}),
    ...(picture ? { picture } : {}),
    ...(text(value["color"]) ? { color: text(value["color"]) } : {}),
    ...(def.command ? { command: def.command } : {}),
    // A plugin's own command in a pane's header is told which pane.
    ...(def.command && pane !== undefined && !def.command.startsWith("lume.")
      ? { args: { pane } }
      : {}),
    ...(def.menu ? { menu: def.menu } : {}),
    ...(rows && rows.length > 0 ? { card: rows } : {}),
    ...(problem === undefined ? {} : { problem }),
    ...(pane === undefined ? {} : { pane }),
  };
  switch (def.kind) {
    case "text":
      return { ...base, kind: "text", text: shownText ?? label };
    case "icon":
      return { ...base, kind: "icon", icon: "dot", label: label || def.id };
    case "meter":
      return {
        ...base,
        kind: "meter",
        label,
        value: number(value["value"]) ?? 0,
        ...(shownText === undefined ? {} : { text: shownText }),
      };
    case "sparkline":
      return {
        ...base,
        kind: "sparkline",
        label,
        points: numbers(value["points"]) ?? [],
        ...(shownText === undefined ? {} : { text: shownText }),
      };
    case "badge":
      return shownText === undefined || shownText === ""
        ? undefined
        : { ...base, kind: "badge", text: shownText };
    default:
      return undefined;
  }
}

/** What the user set one of a plugin's settings to, if anything. */
function settingOf(plugin: string, key: string, view: ConfigView | null): unknown {
  const table = view?.config["plugins"];
  const own =
    typeof table === "object" && table !== null
      ? (table as Record<string, unknown>)[plugin]
      : undefined;
  return typeof own === "object" && own !== null
    ? (own as Record<string, unknown>)[key]
    : undefined;
}

/** Whether the switch of a plugin's settings an item is shown under is on. */
function switchedOn(def: StatusDef, view: ConfigView | null): boolean {
  const named = def.shown_when;
  if (named === undefined) return true;
  const set = settingOf(def.plugin, named.key, view);
  return typeof set === "boolean" ? set : named.default;
}

/**
 * The place an item is in: the one its plugin's setting that chooses it holds, if the
 * user set it to one it may choose, or else the one its manifest gives.
 */
function slotOf(def: StatusDef, view: ConfigView | null): StatusSlot | undefined {
  const chooser = def.slot_setting;
  const set = chooser === undefined ? undefined : settingOf(def.plugin, chooser.key, view);
  const chosen = typeof set === "string" && chooser?.slots.includes(set) === true ? set : def.slot;
  return SLOTS.includes(chosen as StatusSlot) ? (chosen as StatusSlot) : undefined;
}

/** The plugins' items as they are drawn now, under the settings in `view`. */
export function pluginStatusItems(
  state: PluginsState,
  view: ConfigView | null = null,
): StatusItem[] {
  const shown = state.shown;
  if (!shown) return [];
  const items: StatusItem[] = [];
  for (const def of shown.status) {
    if (!holds(def.when, state.contexts) || !switchedOn(def, view)) continue;
    if (def.slot === "pane.header") {
      const prefix = `${def.id}@`;
      for (const [key, values] of state.values) {
        if (!key.startsWith(prefix)) continue;
        const item = itemOf(def, key, values, key.slice(prefix.length), state, view);
        if (item) items.push(item);
      }
      continue;
    }
    const item = itemOf(def, def.id, state.values.get(def.id), undefined, state, view);
    if (item) items.push(item);
  }
  return items;
}

/**
 * Keeps the plugins' items in a window's status registry as the plugins' state
 * changes. Returns how to stop.
 */
export function followPluginStatus(
  plugins: PluginsStore,
  status: StatusStore,
  config: ConfigStore,
): () => void {
  let held = new Map<string, string>();
  const sync = () => {
    const state = plugins.getState();
    const next = new Map<string, string>();
    const registry = status.getState();
    for (const item of pluginStatusItems(state, config.getState().view)) {
      const written = JSON.stringify(item);
      next.set(item.id, written);
      if (held.get(item.id) !== written) registry.register(item);
    }
    for (const id of held.keys()) {
      if (!next.has(id)) registry.remove(id);
    }
    held = next;
  };
  sync();
  const stopPlugins = plugins.subscribe(sync);
  const stopConfig = config.subscribe(sync);
  return () => {
    stopPlugins();
    stopConfig();
  };
}
