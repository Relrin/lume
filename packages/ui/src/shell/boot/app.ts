/** The window's app: everything its components and views work with, put together. */

import type { RenderStats } from "@lume/renderer";

import type { Commands } from "../../commands/run.ts";
import type { CommandStore } from "../../commands/store.ts";
import type { ConfigStore } from "../../config/store.ts";
import type { ConnectStore } from "../../connect/store.ts";
import type { CoreClient } from "../../core/client.ts";
import type { Machine } from "../../core/types.ts";
import type { CardDrag } from "../../drag/controller.ts";
import type { PanelDragStore } from "../../drag/panels.ts";
import type { PaneReports } from "../../events/store.ts";
import type { ForwardedKey } from "../../keys/forwarded.ts";
import type { TabAreaController } from "../../layout/controller.ts";
import type { LayoutStore } from "../../layout/store.ts";
import type { PaneId } from "../../layout/types.ts";
import type { PaletteStore } from "../../palette/store.ts";
import type { PaneTitles } from "../../panes/titles.ts";
import type { PaneViews } from "../../panes/views.ts";
import type { PluginsStore } from "../../plugins/store.ts";
import type { VisibleItems } from "../../plugins/visible.ts";
import type { ProfilesStore } from "../../profiles/store.ts";
import type { RexStore } from "../../rex/store.ts";
import type { SearchController } from "../../search/controller.ts";
import type { StatusStore } from "../../status/registry.ts";
import type { CursorFx } from "../../terminal/cursor-fx.ts";
import type { PromptStore } from "../../terminal/prompts.ts";
import type { WindowStateStore } from "../../window/state.ts";
import type { HostLinkStore } from "../host-link.ts";
import type { NoticeStore } from "../notices.ts";
import type { TabUiStore } from "../tab-ui.ts";
import type { Stores } from "./stores.ts";

/** Everything the window's components and views work with. */
export interface ShellApp {
  readonly layout: LayoutStore;
  readonly config: ConfigStore;
  readonly commandStore: CommandStore;
  readonly client: CoreClient;
  readonly area: TabAreaController;
  readonly views: PaneViews;
  readonly titles: PaneTitles;
  /** What the sessions of the window's panes have reported. */
  readonly reports: PaneReports;
  readonly commands: Commands;
  readonly prompts: PromptStore;
  readonly notices: NoticeStore;
  /** Whether the command palette is on show, and what for. */
  readonly palette: PaletteStore;
  /** Whether the palette that opens a terminal from a profile is on show. */
  readonly connect: ConnectStore;
  /** The ways there are to start a terminal. */
  readonly profiles: ProfilesStore;
  /** The Rex servers there are, and how each stands. */
  readonly rex: RexStore;
  /** What the page knows of the machine it runs on. */
  readonly machine: Machine;
  /** The status items the window's bars show. */
  readonly status: StatusStore;
  /** What the plugins add to the window, and what their code shows in it. */
  readonly plugins: PluginsStore;
  /** Which of the plugins' things the window has on screen. */
  readonly visible: VisibleItems;
  /**
   * Asks a plugin for the rows beneath a row of its list panel that the user opened,
   * by the panel's id and the ids of the rows down to it.
   */
  readonly expandRow: (panel: string, path: readonly string[]) => Promise<void>;
  /** What the window is doing on the desktop. */
  readonly windowState: WindowStateStore;
  /** Whether the session host is connected. */
  readonly hostLink: HostLinkStore;
  /** Moves cards about the window by their headers. */
  readonly cardDrag: CardDrag;
  /** The panel that is being dragged to another side of the window, if one is. */
  readonly panelDrag: PanelDragStore;
  /** The searches of the window's panes. */
  readonly search: SearchController;
  /** What the user is doing with the tabs: renaming one, a menu, a drag. */
  readonly tabUi: TabUiStore;
  /** The cursor's effects, drawn over the whole window. */
  readonly cursorFx: CursorFx;
  /**
   * Takes a key one of the plugins' views passed on, from the view of `pane` if it is
   * in one: it runs what the key is bound to only if that moves about the window.
   */
  forwardKey(key: ForwardedKey, pane?: PaneId): void;
  /**
   * How many renderers of each kind are alive and what their glyphs take, and how much
   * memory the terminals of the window hold between them.
   */
  stats(): RenderStats & { readonly wasmBytes: number };
  /**
   * Ends the page's work: its views, its renderers and what it listens to. The window
   * never does this; a test that starts a page more than once does, between two.
   */
  dispose(): void;
}

/** The parts of the app that are not stores. */
export interface AppParts {
  readonly stores: Stores;
  readonly client: CoreClient;
  readonly machine: Machine;
  readonly area: TabAreaController;
  readonly views: PaneViews;
  readonly commands: Commands;
  readonly prompts: PromptStore;
  readonly notices: NoticeStore;
  readonly visible: VisibleItems;
  readonly cardDrag: CardDrag;
  readonly search: SearchController;
  readonly cursorFx: CursorFx;
  readonly forwardKey: ShellApp["forwardKey"];
  readonly stats: ShellApp["stats"];
  readonly dispose: () => void;
}

export function buildApp(parts: AppParts): ShellApp {
  const { stores, client } = parts;
  return {
    layout: stores.layout,
    config: stores.config,
    commandStore: stores.commandStore,
    client,
    area: parts.area,
    views: parts.views,
    titles: stores.titles,
    reports: stores.reports,
    commands: parts.commands,
    prompts: parts.prompts,
    notices: parts.notices,
    palette: stores.palette,
    connect: stores.connect,
    profiles: stores.profiles,
    rex: stores.rex,
    machine: parts.machine,
    status: stores.status,
    plugins: stores.plugins,
    visible: parts.visible,
    expandRow: (panel, path) => client.uiPanelExpand(panel, path),
    windowState: stores.windowState,
    hostLink: stores.hostLink,
    cardDrag: parts.cardDrag,
    panelDrag: stores.panelDrag,
    search: parts.search,
    tabUi: stores.tabUi,
    cursorFx: parts.cursorFx,
    forwardKey: parts.forwardKey,
    stats: parts.stats,
    dispose: parts.dispose,
  };
}
