/**
 * Starting the window's page: once for each page, however often a component mounts.
 *
 * What is slow or can be had only once is done here: loading the terminal engine, asking
 * for a graphics device, opening the carrier. In development React mounts components
 * twice, and a second carrier would retire the first, so none of this may follow a
 * component's life.
 *
 * The page owns nothing that lasts. The layout, the configuration and the commands are
 * the core's; the page asks for each once and follows what it is told from then on. A
 * page that is reloaded starts here again and finds every session still running.
 */

import { RenderHost, TIER_ORDER, type RenderStats, type Tier } from "@lume/renderer";
import { DEFAULT_TOKENS, terminalAppearance } from "@lume/tokens";
import { KeyEncoder, Vt } from "@lume/vt";
import wasmUrl from "@lume/vt/ghostty-vt.wasm?url";

import { Commands } from "../commands/run.ts";
import { createCommandStore, type CommandStore } from "../commands/store.ts";
import { applyConfig, describeProblem } from "../config/apply.ts";
import { loadTerminalFonts } from "../config/fonts.ts";
import { createConfigStore, type ConfigStore } from "../config/store.ts";
import { CoreClient } from "../core/client.ts";
import type { ConfigView, Notice } from "../core/types.ts";
import { checkFrameContract } from "../frame-contract.ts";
import { CardDrag } from "../drag/controller.ts";
import { createPanelDragStore, type PanelDragStore } from "../drag/panels.ts";
import { Shortcuts } from "../keys/shortcuts.ts";
import { TabAreaController } from "../layout/controller.ts";
import { estimatePane, gridFor } from "../layout/estimate.ts";
import { panesOf, selectActiveTab, selectFocus } from "../layout/selectors.ts";
import { createLayoutStore, type LayoutStore } from "../layout/store.ts";
import type { Intent, PaneId } from "../layout/types.ts";
import { createPaletteStore, type PaletteStore } from "../palette/store.ts";
import { PaneTitles } from "../panes/titles.ts";
import { PaneViews } from "../panes/views.ts";
import { SearchController } from "../search/controller.ts";
import { createStatusStore, type StatusStore } from "../status/registry.ts";
import { currentPlatform, KeyLayout } from "../terminal/keyboard.ts";
import { PromptStore } from "../terminal/prompts.ts";
import { answerRequest, type PaneRef, type RemoteTarget } from "../terminal/remote.ts";
import type { TerminalView } from "../terminal/view.ts";
import { DEFAULT_APPEARANCE, rendererTheme, type Appearance } from "../theme.ts";
import { TauriCarrier, type Carrier } from "../transport/carrier.ts";
import { CommandError } from "../transport/types.ts";
import { FONT_SIZE_MAX_PX, FONT_SIZE_MIN_PX } from "../tuning.ts";
import { createWindowStateStore, type WindowStateStore } from "../window/state.ts";
import { closeRequested, mayRun } from "./closing.ts";
import { focusBar } from "./focus-bar.ts";
import { NoticeStore, sentence } from "./notices.ts";

/** The page's prompts. Created with the module so a component can show them at once. */
export const prompts = new PromptStore();

/** The page's notices. Created with the module, like the prompts. */
export const notices = new NoticeStore();

/** Everything the window's components and views work with. */
export interface ShellApp {
  readonly layout: LayoutStore;
  readonly config: ConfigStore;
  readonly commandStore: CommandStore;
  readonly client: CoreClient;
  readonly area: TabAreaController;
  readonly views: PaneViews;
  readonly titles: PaneTitles;
  readonly commands: Commands;
  readonly prompts: PromptStore;
  readonly notices: NoticeStore;
  /** Whether the command palette is on show, and what for. */
  readonly palette: PaletteStore;
  /** The status items the window's bars show. */
  readonly status: StatusStore;
  /** What the window is doing on the desktop. */
  readonly windowState: WindowStateStore;
  /** Moves cards about the window by their headers. */
  readonly cardDrag: CardDrag;
  /** The panel that is being dragged to another side of the window, if one is. */
  readonly panelDrag: PanelDragStore;
  /** The searches of the window's panes. */
  readonly search: SearchController;
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

/** The parts of the keyboard-layout interface some webviews offer. */
interface KeyboardLike {
  getLayoutMap(): Promise<Iterable<[string, string]>>;
}

/** The keyboard layout as the webview reports it, where it can. */
async function keyboardLayout(): Promise<KeyLayout> {
  const keyboard = (navigator as Navigator & { keyboard?: KeyboardLike }).keyboard;
  try {
    if (keyboard) return new KeyLayout(new Map(await keyboard.getLayoutMap()));
  } catch {
    // Not allowed or not supported: the layout is learnt from what is typed instead.
  }
  return new KeyLayout();
}

/** The tiers the configuration lets a terminal be drawn with, best first. */
function tiersFor(view: ConfigView): readonly Tier[] {
  const tier = view.config.renderer.tier;
  return tier === "auto" ? TIER_ORDER : [tier];
}

/** No context holds besides the ones a key's place gives it. */
const NO_CONTEXT: ReadonlySet<string> = new Set();

/** The context of a key pressed while the command palette is on show. */
const PALETTE_CONTEXT: ReadonlySet<string> = new Set(["palette"]);

/** A pane's search bar, as an element is asked whether it is in one. */
const SEARCH_BAR = "[data-search-bar]";

/**
 * The parts of the window that keep the keys pressed in them: a terminal offers its
 * own keys to the bindings, and a dialog or a menu takes what is typed in it.
 */
const KEEPS_KEYS = ".terminal, [role=alertdialog], [role=menu]";

/** The title the window has when no program has given the focused pane one. */
const DEFAULT_TITLE = "Lume";

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Starts a window's page on `carrier`. The application's own page does this once, on
 * the carrier into the core; a test starts one on a carrier of its own.
 */
export async function startShell(carrier: Carrier): Promise<ShellApp> {
  checkFrameContract();
  const layout = createLayoutStore();
  const config = createConfigStore();
  const commandStore = createCommandStore();
  const titles = new PaneTitles();
  const palette = createPaletteStore();
  const status = createStatusStore();
  const windowState = createWindowStateStore();
  const panelDrag = createPanelDragStore();

  // What the core sends before the page has its first copies is kept and followed
  // once it has them; a change the copies already hold is passed over there.
  let early: Notice[] | null = [];
  let onNotice = (notice: Notice): void => void early?.push(notice);
  let ready: () => void = () => undefined;
  const started = new Promise<void>((resolve) => (ready = resolve));
  let onRequest = (message: unknown): void => void started.then(() => onRequest(message));

  // The graphics device is asked for before the configuration is known, since it is
  // the slowest part of a start. A configuration that names one tier is rare.
  const hostStarted = RenderHost.create();
  const [vt, keyLayout, client] = await Promise.all([
    Vt.instantiate(fetch(wasmUrl)),
    keyboardLayout(),
    CoreClient.open(carrier, {
      notice: (notice) => onNotice(notice),
      request: (message) => onRequest(message),
      failure: (error) =>
        prompts.notice(`The application can no longer be reached: ${error.message}`),
    }),
  ]);
  const [layoutView, configView, commandList] = await Promise.all([
    client.layoutView(),
    client.configView(),
    client.commandsList(),
  ]);
  let host = await hostStarted;
  if (configView.config.renderer.tier !== "auto") {
    host.dispose();
    host = await RenderHost.create({ tiers: tiersFor(configView) });
  }

  config.getState().set(configView);
  commandStore.getState().set(commandList);
  applyConfig(configView);

  const appearance = (): Appearance => {
    const view = config.getState().view;
    return view ? terminalAppearance(view.tokens, view.theme.scheme) : DEFAULT_APPEARANCE;
  };
  // The look is worked out once for each configuration, not once for each view.
  let look = appearance();
  // A terminal sizes its cells and draws its glyphs once: the font is there first.
  await loadTerminalFonts(look);

  const area = new TabAreaController({
    layout,
    config: () => config.getState().view,
    apply: (intent) => client.layoutApply(intent),
    defaults: DEFAULT_TOKENS,
  });

  const platform = currentPlatform();
  // Running a command needs the views, and the views need the shortcuts.
  const shortcuts = new Shortcuts({ platform, layout: keyLayout }, (command, args) =>
    commands.run(command, args),
  );
  shortcuts.setBindings(configView.keymap.bindings);

  const views = new PaneViews({
    shared: {
      vt,
      host,
      keys: new KeyEncoder(vt),
      platform,
      layout: keyLayout,
      prompts,
      clipboard: {
        read: () => client.clipboardRead(),
        write: (text) => client.clipboardWrite(text),
      },
      openLink: (url) => client.openLink(url),
      developerTools: import.meta.env.DEV,
    },
    layout,
    client,
    area,
    titles,
    config: () => config.getState().view,
    appearance: () => look,
    shortcut: (pane, event, alternateScreen) => {
      // While the palette is on show every key is the palette's, wherever the
      // keyboard happens to be: none reaches a program.
      if (palette.getState().shown) {
        shortcuts.handle(event, PALETTE_CONTEXT);
        return true;
      }
      const contexts = new Set(["terminal"]);
      if (alternateScreen) contexts.add("alt-screen");
      if (layout.getState().floats[pane]) contexts.add("floating");
      return shortcuts.handle(event, contexts);
    },
    keyboard: (pane) => search.setKeyboard(pane, "terminal"),
    searched: (pane) => search.touched(pane),
  });
  // A search works in stretches with the page's other work between them. A message to
  // oneself is the soonest a task can be had that still lets a frame or a key in
  // first: a timer of no length is made to wait some milliseconds.
  const turns = new MessageChannel();
  const waiting: (() => void)[] = [];
  turns.port1.onmessage = () => waiting.shift()?.();
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
  const commands = new Commands({
    store: commandStore,
    client,
    views,
    palette,
    search,
    mayRun: (id, args) => mayRun({ layout, config, views, prompts, client, notices }, id, args),
    focusBar: () => focusBar(),
    failed: (message) => notices.push("error", sentence(message)),
  });
  const cardDrag = new CardDrag({
    layout,
    area,
    apply: (intent) => client.layoutApply(intent),
    failed: (message) => notices.push("error", sentence(message)),
  });

  // Following the core.

  let refetching = false;
  const refetch = (): void => {
    if (refetching) return;
    refetching = true;
    client.layoutView().then(
      (view) => {
        refetching = false;
        layout.getState().reset(view);
      },
      (error: unknown) => {
        refetching = false;
        notices.push("error", `The layout could not be read again: ${reason(error)}`);
      },
    );
  };
  const follow = (notice: Notice): void => {
    switch (notice.type) {
      case "layout_changed": {
        const followed = layout.getState().apply(notice.epoch, notice.revision, notice.changes);
        // Changes are missing between the copy and these: the whole is asked for.
        if (followed === "gap") refetch();
        break;
      }
      case "layout_reset":
        layout.getState().reset(notice);
        break;
      case "config_changed":
        if (!config.getState().set(notice.view)) break;
        look = appearance();
        applyConfig(notice.view);
        shortcuts.setBindings(notice.view.keymap.bindings);
        {
          // The terminals are set up again once the font the configuration now names
          // is ready, unless the configuration has changed again by then.
          const settled = look;
          void loadTerminalFonts(settled).then(() => {
            if (look !== settled) return;
            views.configure();
            area.layout();
          });
        }
        break;
      case "config_error":
        notices.push(
          "error",
          `The configuration was not applied, and the one before it is still in use. ${notice.problems
            .map(describeProblem)
            .join(" ")}`,
        );
        break;
      case "window_state":
        windowState.getState().set(notice);
        break;
      case "close_requested":
        void closeRequested({ layout, config, views, prompts, client, notices });
        break;
      case "notice":
        notices.push(notice.level, notice.message);
        break;
    }
  };

  // What the core may ask.

  const viewOf = (target: PaneRef): { readonly pane: PaneId; readonly view: TerminalView } => {
    if (target.pane !== undefined) {
      const view = views.get(target.pane);
      if (!view) throw new CommandError("not_shown", `this window has no pane ${target.pane}`);
      return { pane: target.pane, view };
    }
    const found = target.session === undefined ? views.focused() : views.bySession(target.session);
    if (!found) {
      throw new CommandError(
        "not_shown",
        target.session === undefined
          ? "no pane of this window has the keyboard"
          : `session ${target.session} is in no pane of this window`,
      );
    }
    return found;
  };
  const target: RemoteTarget = {
    estimate: (intent: Intent) => {
      const box = area.box();
      if (!box) throw new CommandError("no_window", "the window has not been laid out yet");
      const state = layout.getState();
      const rect = estimatePane(state, intent, box, area.gapPx());
      if (!rect) throw new CommandError("bad_request", "the intent makes no pane");
      // A pane made by dividing another shares its tab; a new tab's pane is alone,
      // and the scratch terminal floats.
      const floating = intent.type === "scratch_show";
      let beside = 1;
      if (intent.type === "split") {
        const tab = state.tabs[state.paneTabs[intent.pane] ?? ""];
        beside = (tab ? panesOf(tab.root).length : 1) + 1;
      }
      const fontSizePx = Math.min(
        Math.max(look.fontSizePx + views.zoomPx, FONT_SIZE_MIN_PX),
        FONT_SIZE_MAX_PX,
      );
      const dpr = window.devicePixelRatio;
      const metrics = host.measureCells(rendererTheme(look, fontSizePx), dpr);
      return gridFor(rect, area.chrome(beside, floating), metrics, dpr);
    },
    command: (id, args) => {
      commands.runInPage(id, args);
      return Promise.resolve();
    },
    // The core asks about a pane as soon as it has attached this page to the pane's
    // session, which may be before the page has heard that it is attached.
    screen: async (ref) => {
      const { pane, view } = viewOf(ref);
      await views.settled(pane);
      return { pane, ...view.screen() };
    },
    waitText: async (ref, text, timeoutMs) => {
      const { pane, view } = viewOf(ref);
      await views.settled(pane);
      return { pane, ...(await view.waitText(text, timeoutMs)) };
    },
    resizeTo: async (ref, size) => {
      const { pane, view } = viewOf(ref);
      await views.settled(pane);
      return view.resizeTo(size);
    },
  };

  // The keyboard and the window's title follow the layout's focus.

  // While the palette is on show the keyboard is the palette's, and when the palette
  // is put away the keyboard goes back to where the user was working: the terminal of
  // the focused pane, or that pane's search bar if they were typing there.
  const refocus = (): void => {
    if (palette.getState().shown) return;
    const pane = selectFocus(layout.getState());
    if (pane !== null && search.focusBar(pane)) return;
    views.focus();
  };
  let paletteShown = false;
  palette.subscribe((state) => {
    if (state.shown === paletteShown) return;
    paletteShown = state.shown;
    if (!paletteShown) queueMicrotask(refocus);
  });
  let focus: PaneId | null = null;
  let active: string | null = null;
  const retitle = (): void => {
    document.title = (focus === null ? undefined : titles.get(focus)) ?? DEFAULT_TITLE;
  };
  let epoch: number | undefined;
  layout.subscribe((state) => {
    if (!state.ready) return;
    // The searches follow the panes: another layout altogether has none of them, and
    // a pane that is gone takes its search with it.
    if (epoch !== undefined && epoch !== state.epoch) search.reset();
    epoch = state.epoch;
    queueMicrotask(() => search.prune(views.panes()));
    const now = selectFocus(state);
    const tab = selectActiveTab(state);
    if (now === focus && tab === active) return;
    focus = now;
    active = tab;
    retitle();
    // The card of a pane in a tab that has just been shown is rendered after this.
    queueMicrotask(refocus);
  });
  titles.subscribe((pane) => {
    if (pane === focus) retitle();
  });
  const onWindowFocus = (): void => {
    // The keyboard comes back to the terminal unless something else in the window
    // was using it: a dialog, or a field being typed in.
    const using = document.activeElement;
    if (using === null || using === document.body) refocus();
  };
  window.addEventListener("focus", onWindowFocus);
  // Keys pressed while the keyboard is with the window's own controls, not with a
  // terminal, are offered to the bindings that hold everywhere, and to those that
  // hold where the keyboard is.
  const onWindowKey = (event: KeyboardEvent): void => {
    const where = event.target;
    if (!(where instanceof Element)) return;
    if (where.closest(KEEPS_KEYS)) return;
    let contexts = NO_CONTEXT;
    if (palette.getState().shown) {
      contexts = PALETTE_CONTEXT;
    } else if (where.closest(SEARCH_BAR)) {
      // A search bar is not its terminal: a key that copies from a terminal, say,
      // must be left to the field.
      const pane = where.closest<HTMLElement>("[data-pane]")?.dataset.pane;
      contexts = new Set(["search"]);
      if (pane !== undefined && layout.getState().floats[pane]) {
        contexts = new Set(["search", "floating"]);
      }
    }
    if (shortcuts.handle(event, contexts)) {
      event.preventDefault();
      event.stopPropagation();
    }
  };
  window.addEventListener("keydown", onWindowKey, true);

  // The window's own buttons show what the window is doing, which only the core knows.
  client.windowState().then(
    (state) => windowState.getState().set(state),
    () => undefined,
  );

  layout.getState().reset(layoutView);
  onNotice = follow;
  for (const notice of early ?? []) follow(notice);
  early = null;
  onRequest = (message) => {
    void answerRequest(target, message).then((answer) => {
      if (!answer) return;
      client.answer(answer.id, answer.result).catch((error: unknown) => {
        console.error("an answer to the core was not delivered", error);
      });
    });
  };
  ready();

  return {
    layout,
    config,
    commandStore,
    client,
    area,
    views,
    titles,
    commands,
    prompts,
    notices,
    palette,
    status,
    windowState,
    cardDrag,
    panelDrag,
    search,
    stats: () => ({ ...host.stats(), wasmBytes: vt.memoryBytes }),
    dispose: () => {
      window.removeEventListener("focus", onWindowFocus);
      window.removeEventListener("keydown", onWindowKey, true);
      onNotice = () => undefined;
      onRequest = () => undefined;
      search.reset();
      turns.port1.close();
      views.dispose();
      host.dispose();
    },
  };
}

let app: Promise<ShellApp> | undefined;

/** The window's page. The first call starts it; every call gets the same one. */
export function shellApp(): Promise<ShellApp> {
  app ??= startShell(new TauriCarrier());
  return app;
}
