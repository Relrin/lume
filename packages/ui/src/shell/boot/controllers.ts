/**
 * The window's controllers: the panes' views, their searches, the commands and moving
 * cards about. They need each other, so each is handed the cells of those made after it.
 */

import type { RenderHost } from "@lume/renderer";
import { KeyEncoder, type Vt } from "@lume/vt";

import { Commands } from "../../commands/run.ts";
import { connectionDialog } from "../../connections/store.ts";
import type { CoreClient } from "../../core/client.ts";
import { CardDrag } from "../../drag/controller.ts";
import type { TabAreaController } from "../../layout/controller.ts";
import { selectActiveTab, selectFocus } from "../../layout/selectors.ts";
import type { PaneId } from "../../layout/types.ts";
import { PaneViews } from "../../panes/views.ts";
import { openersFor } from "../../plugins/openers.ts";
import { rexHostNamed } from "../../rex/model.ts";
import { SearchController } from "../../search/controller.ts";
import { ServedBudget } from "../../terminal/budget.ts";
import type { CursorFx } from "../../terminal/cursor-fx.ts";
import type { KeyLayout } from "../../terminal/keyboard.ts";
import type { PromptStore } from "../../terminal/prompts.ts";
import type { Appearance } from "../../theme.ts";
import { mayRun, type ClosingApp } from "../closing.ts";
import { focusBar } from "../focus-bar.ts";
import { sentence, type NoticeStore } from "../notices.ts";
import { folderDialog } from "../open-folder.ts";
import type { Disposer } from "./disposer.ts";
import type { Input } from "./input.ts";
import type { Late } from "./late.ts";
import type { Stores } from "./stores.ts";

export interface ViewsDeps {
  readonly stores: Stores;
  readonly vt: Vt;
  readonly host: RenderHost;
  readonly cursorFx: CursorFx;
  readonly keyLayout: KeyLayout;
  readonly client: CoreClient;
  readonly area: TabAreaController;
  readonly prompts: PromptStore;
  readonly input: Input;
  readonly look: () => Appearance;
  readonly appearanceOf: (pane: PaneId) => Appearance;
  /** The views take the searches' part in what the keyboard does. */
  readonly search: Late<SearchController>;
}

/** The views of the window's panes: their terminals, and how keys reach them. */
export function createViews(deps: ViewsDeps): PaneViews {
  const { stores, vt, host, client, input, search } = deps;
  const { layout, config, plugins, tabUi } = stores;
  return new PaneViews({
    shared: {
      vt,
      host,
      cursor: deps.cursorFx,
      keys: new KeyEncoder(vt),
      budget: new ServedBudget(() => vt.memoryBytes),
      platform: input.platform,
      layout: deps.keyLayout,
      prompts: deps.prompts,
      clipboard: {
        read: () => client.clipboardRead(),
        write: (text) => client.clipboardWrite(text),
      },
      // A link one of the plugins' views offers to open is the application's to
      // decide about, with the user where there is a choice.
      openLink: (url) =>
        openersFor(url, plugins.getState().shown?.openers ?? []).length > 0
          ? client.commandRun("lume.app.open", { uri: url })
          : client.openLink(url),
      developerTools: import.meta.env.DEV,
    },
    layout,
    client,
    area: deps.area,
    titles: stores.titles,
    reports: stores.reports,
    config: () => config.getState().view,
    appearance: deps.look,
    appearanceOf: deps.appearanceOf,
    shortcut: (pane, event, alternateScreen) => {
      // While a palette is on show every key is the palette's, wherever the keyboard
      // happens to be: none reaches a program.
      const overlay = input.overlayContext();
      if (overlay) {
        input.shortcuts.handle(event, overlay);
        return true;
      }
      const contexts = new Set(["terminal"]);
      if (alternateScreen) contexts.add("alt-screen");
      if (layout.getState().floats[pane]) contexts.add("floating");
      return input.shortcuts.handle(event, input.withPlugins(contexts));
    },
    menu: (pane, at, selection) =>
      tabUi.getState().open({
        kind: "terminal",
        pane,
        anchor: { left: at.clientX, top: at.clientY, bottom: at.clientY },
        selection,
      }),
    keyboard: (pane) => search.get().setKeyboard(pane, "terminal"),
    searched: (pane) => search.get().touched(pane),
  });
}

/**
 * The searches of the window's panes. A search works in stretches with the page's
 * other work between them. A message to oneself is the soonest a task can be had that
 * still lets a frame or a key in first: a timer of no length is made to wait some
 * milliseconds.
 */
export function createSearch(views: PaneViews, disposer: Disposer): SearchController {
  const turns = new MessageChannel();
  const waiting: (() => void)[] = [];
  turns.port1.onmessage = () => waiting.shift()?.();
  disposer.add(() => turns.port1.close());
  const search = new SearchController({
    target: (pane) => views.get(pane),
    soon: (task) => {
      waiting.push(task);
      turns.port2.postMessage(null);
    },
    setTimer: (task, ms) => window.setTimeout(task, ms),
    clearTimer: (timer) => window.clearTimeout(timer),
    now: () => performance.now(),
  });
  disposer.add(() => search.reset());
  return search;
}

export interface CommandsDeps {
  readonly stores: Stores;
  readonly client: CoreClient;
  readonly views: PaneViews;
  readonly search: SearchController;
  readonly prompts: PromptStore;
  readonly notices: NoticeStore;
  /** What the commands that close things ask about the window. */
  readonly closing: ClosingApp;
}

/** The commands the window runs, its own and the core's. */
export function createCommands(deps: CommandsDeps): Commands {
  const { stores, client, views, search, prompts, notices, closing } = deps;
  const { layout, rex, tabUi } = stores;
  return new Commands({
    store: stores.commandStore,
    client,
    views,
    palette: stores.palette,
    connect: stores.connect,
    search,
    mayRun: (id, args) => mayRun(closing, id, args),
    focusBar: () => focusBar(),
    renameTab: (tab) => tabUi.getState().startRename(tab ?? selectActiveTab(layout.getState())),
    renamePane: (named) => {
      const pane = named ?? selectFocus(layout.getState());
      if (pane === null || pane === undefined) return;
      // The field is in the pane's header. A pane alone in its tab may have none, and
      // is what its tab is called after: there the field is the tab's.
      if (views.hasHeader(pane)) tabUi.getState().startPaneRename(pane);
      else tabUi.getState().startRename(layout.getState().paneTabs[pane] ?? null);
    },
    answerPrompt: (proceed, item) => prompts.answerCurrent(proceed, item),
    openFolder: () => folderDialog.getState().show(false),
    addConnection: (kind, address) => connectionDialog.getState().add(kind, true, address),
    rexHost: (named) => rexHostNamed(rex.getState().list, named),
    failed: (message) => notices.push("error", sentence(message)),
  });
}

/** Moves cards about the window by their headers. */
export function createCardDrag(
  stores: Stores,
  area: TabAreaController,
  client: CoreClient,
  notices: NoticeStore,
): CardDrag {
  return new CardDrag({
    layout: stores.layout,
    area,
    apply: (intent) => client.layoutApply(intent),
    failed: (message) => notices.push("error", sentence(message)),
  });
}
