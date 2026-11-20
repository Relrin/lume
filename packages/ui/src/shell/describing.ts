/**
 * What the window's chrome shows of a pane, read from wherever it is kept: the pane's
 * reports, the layout, the profiles and the configuration.
 *
 * A header, a tab or a marker that shows a pane follows it here. It is told whenever
 * something it reads may have changed and writes what it shows itself, so a pane's
 * reports never render the component tree.
 */

import type { ConfigView } from "../core/types.ts";
import { greatest, type Activity } from "../events/activity.ts";
import {
  describePane,
  paneLabel,
  type DescribeContext,
  type PaneDescription,
  type PaneFacts,
} from "../events/describe.ts";
import { iconChoices, type PaneIcon, type ProgramIcon } from "../events/programs.ts";
import { isTabMuted, panesOf, selectTitlePane, UNTITLED_TAB } from "../layout/selectors.ts";
import type { LayoutState } from "../layout/store.ts";
import type { PaneId, TabId } from "../layout/types.ts";
import { TAB_ICONS_MAX } from "../tuning.ts";
import type { ShellApp } from "./boot.ts";

/** The user's pictures for programs, worked out once for each configuration. */
const ICONS = new WeakMap<ConfigView, ReadonlyMap<string, ProgramIcon>>();

const NO_ICONS: ReadonlyMap<string, ProgramIcon> = new Map();

function iconsOf(view: ConfigView | null): ReadonlyMap<string, ProgramIcon> {
  if (!view) return NO_ICONS;
  let icons = ICONS.get(view);
  if (!icons) {
    icons = iconChoices(view.config.appearance["program-icons"]);
    ICONS.set(view, icons);
  }
  return icons;
}

/** What describing a pane reads besides the pane, as the window holds it now. */
export function describeContext(app: ShellApp): DescribeContext {
  const view = app.config.getState().view;
  return {
    machine: app.machine,
    windows: view?.platform === "windows",
    profiles: app.profiles.getState().list,
    icons: iconsOf(view),
  };
}

/** What the layout says of a pane. */
export function paneFacts(state: LayoutState, pane: PaneId): PaneFacts {
  const entry = state.panes[pane] ?? state.scratch?.parked?.pane;
  const content = entry?.id === pane ? entry.content : undefined;
  const terminal = content?.type === "terminal" ? content : undefined;
  // A pane that shows a plugin's view is called after the view until it is named.
  const name =
    entry?.id === pane
      ? (entry.title ?? (content?.type === "plugin_view" ? content.view : null))
      : null;
  return {
    name: name === null || name === undefined || name.trim() === "" ? null : name,
    startCwd: terminal?.cwd ?? null,
    launcher: terminal?.launcher ?? null,
    profile: terminal?.profile ?? null,
  };
}

/** A pane as the chrome shows it now. */
export function describeNow(app: ShellApp, pane: PaneId): PaneDescription {
  return describePane(
    app.reports.get(pane),
    paneFacts(app.layout.getState(), pane),
    describeContext(app),
  );
}

/**
 * Writes one of an element's marks, `data-<name>`, or takes it away when it says only
 * what is so of every pane: a mark is there to be styled by, and to be noticed.
 */
export function mark(element: HTMLElement, name: string, value: string, usual: string): void {
  if (value === usual) delete element.dataset[name];
  else if (element.dataset[name] !== value) element.dataset[name] = value;
}

/** A tab as the chrome shows it: what it is called, its programs' pictures, what it gathered. */
export interface TabDescription {
  readonly label: string;
  /** The pictures of the programs of its panes, the one it is named after first. */
  readonly icons: readonly PaneIcon[];
  readonly activity: Activity;
}

/** A tab as the chrome shows it now. */
export function describeTabNow(app: ShellApp, tab: TabId): TabDescription {
  const state = app.layout.getState();
  const entry = state.tabs[tab];
  const named = selectTitlePane(state, tab);
  const panes = entry ? panesOf(entry.root) : [];
  const ordered = named === undefined ? panes : [named, ...panes.filter((pane) => pane !== named)];
  const context = describeContext(app);
  const described = ordered.map((pane) =>
    describePane(app.reports.get(pane), paneFacts(state, pane), context),
  );
  const first = described[0];
  const own = named === undefined ? null : paneFacts(state, named).name;
  const label = entry?.title || own || (first ? paneLabel(first) : UNTITLED_TAB);
  return {
    label,
    icons: described.slice(0, TAB_ICONS_MAX).map((description) => description.icon),
    // A muted tab is not marked as waiting, whatever its panes say.
    activity: isTabMuted(state, tab)
      ? "idle"
      : greatest(described.map((description) => description.activity)),
  };
}

/**
 * Calls `write` with a tab's description now and whenever it may have changed. Returns
 * how to stop.
 */
export function followTab(
  app: ShellApp,
  tab: TabId,
  write: (description: TabDescription) => void,
): () => void {
  const again = () => write(describeTabNow(app, tab));
  again();
  const stops = [
    app.reports.subscribeAll((pane) => {
      if (app.layout.getState().paneTabs[pane] === tab) again();
    }),
    app.layout.subscribe(again),
    app.profiles.subscribe(again),
    app.config.subscribe(again),
  ];
  return () => stops.forEach((stop) => stop());
}

/**
 * Calls `write` with a pane's description now and whenever it may have changed. Returns
 * how to stop. `write` is called again with the same description when something else
 * changed; whoever writes compares.
 */
export function followPane(
  app: ShellApp,
  pane: PaneId,
  write: (description: PaneDescription) => void,
): () => void {
  const again = () => write(describeNow(app, pane));
  again();
  const stops = [
    app.reports.subscribe(pane, again),
    app.layout.subscribe((state, before) => {
      if (state.panes[pane] !== before.panes[pane] || state.scratch !== before.scratch) again();
    }),
    app.profiles.subscribe(again),
    app.config.subscribe(again),
  ];
  return () => stops.forEach((stop) => stop());
}
