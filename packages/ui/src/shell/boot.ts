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
import { createConnectStore, type ConnectStore } from "../connect/store.ts";
import { CoreClient } from "../core/client.ts";
import type { ConfigView, Machine, Notice } from "../core/types.ts";
import { checkFrameContract } from "../frame-contract.ts";
import { shownAgain, startStep } from "../start-marks.ts";
import { CardDrag } from "../drag/controller.ts";
import { createPanelDragStore, type PanelDragStore } from "../drag/panels.ts";
import { Shortcuts } from "../keys/shortcuts.ts";
import { TabAreaController } from "../layout/controller.ts";
import { estimatePane, gridFor } from "../layout/estimate.ts";
import { panesOf, selectActiveTab, selectFocus } from "../layout/selectors.ts";
import { createLayoutStore, type LayoutStore } from "../layout/store.ts";
import type { Intent, PaneId } from "../layout/types.ts";
import { createPaletteStore, type PaletteStore } from "../palette/store.ts";
import { PaneReports } from "../events/store.ts";
import { PaneLooks } from "../panes/looks.ts";
import { PaneTitles } from "../panes/titles.ts";
import { PaneViews } from "../panes/views.ts";
import { setMotionSource } from "../motion/slots.ts";
import { deliverViewEvent } from "../plugins/frame/registry.ts";
import { followPluginStatus } from "../plugins/status.ts";
import { createPluginsStore, type PluginsStore } from "../plugins/store.ts";
import { VisibleItems } from "../plugins/visible.ts";
import { createProfilesStore, findProfile, type ProfilesStore } from "../profiles/store.ts";
import { SearchController } from "../search/controller.ts";
import { createStatusStore, type StatusStore } from "../status/registry.ts";
import { assets } from "../plugins/assets.ts";
import { CursorFx, cursorFxSettings, cursorFxShaders } from "../terminal/cursor-fx.ts";
import { currentPlatform, KeyLayout } from "../terminal/keyboard.ts";
import { PromptStore } from "../terminal/prompts.ts";
import { answerRequest, type PaneRef, type RemoteTarget } from "../terminal/remote.ts";
import type { TerminalView } from "../terminal/view.ts";
import { DEFAULT_APPEARANCE, rendererTheme, type Appearance } from "../theme.ts";
import { TauriCarrier, type Carrier } from "../transport/carrier.ts";
import { CommandError } from "../transport/types.ts";
import { FONT_SIZE_MAX_PX, FONT_SIZE_MIN_PX } from "../tuning.ts";
import { createWindowStateStore, type WindowStateStore } from "../window/state.ts";
import { closeRequested, confirmRequested, mayRun } from "./closing.ts";
import { describeTabNow, paneFacts } from "./describing.ts";
import { focusBar } from "./focus-bar.ts";
import { NoticeStore, sentence } from "./notices.ts";
import { createTabUiStore, type TabUiStore } from "./tab-ui.ts";

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

/** The context of a key pressed while the Connect palette is on show. */
const CONNECT_CONTEXT: ReadonlySet<string> = new Set(["connect"]);

/** A machine the page was told nothing about. */
const UNKNOWN_MACHINE: Machine = { name: null, home: null };

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
  const connect = createConnectStore();
  const profiles = createProfilesStore();
  const status = createStatusStore();
  const plugins = createPluginsStore();
  const windowState = createWindowStateStore();
  // The user looks at the pane that has the keyboard, while the window is the one in use.
  const attended = (pane: PaneId): boolean => {
    const state = layout.getState();
    return windowState.getState().focused && state.ready && selectFocus(state) === pane;
  };
  const reports = new PaneReports(attended);
  const looks = new PaneLooks();
  /** The look of a pane's terminal: the window's, or the one its profile asks for. */
  const appearanceOf = (pane: PaneId): Appearance => {
    const report = reports.get(pane);
    const list = profiles.getState().list;
    // A session says what it was started from; a pane yet to have one runs what it
    // names, or the default.
    const id =
      report.session !== null
        ? report.profile
        : (paneFacts(layout.getState(), pane).profile ?? list?.default ?? undefined);
    return looks.of(look, findProfile(list, id), list?.looks);
  };
  const panelDrag = createPanelDragStore();

  // The window says which of the plugins' things it has on screen; none while it is
  // out of sight.
  const visible = new VisibleItems((items) => {
    made.client?.uiVisible(items).catch(() => undefined);
  });
  const onVisibility = () => visible.setShown(document.visibilityState !== "hidden");
  document.addEventListener("visibilitychange", onVisibility);
  onVisibility();

  // What the core sends before the page has its first copies is kept and followed
  // once it has them; a change the copies already hold is passed over there.
  // What is made further on, for what is called back before it is.
  const made: { shell?: ShellApp; views?: PaneViews; client?: CoreClient } = {};
  let early: Notice[] | null = [];
  let onNotice = (notice: Notice): void => void early?.push(notice);
  let ready: () => void = () => undefined;
  const started = new Promise<void>((resolve) => (ready = resolve));
  let onRequest = (message: unknown): void => void started.then(() => onRequest(message));

  // The graphics device is asked for before the configuration is known, since it is
  // the slowest part of a start, and nothing waits for it: terminals are drawn with the
  // next tier until it is granted. A configuration that names one tier is rare.
  let host = RenderHost.start();
  const [vt, keyLayout, client] = await Promise.all([
    Vt.instantiate(fetch(wasmUrl)).then((vt) => {
      startStep("vt");
      return vt;
    }),
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
  if (configView.config.renderer.tier !== "auto") {
    host.dispose();
    host = await RenderHost.create({ tiers: tiersFor(configView) });
  }
  void host.gpuSettled.then(() => startStep("gpu"));

  config.getState().set(configView);
  commandStore.getState().set(commandList);
  applyConfig(configView);
  const machine = layoutView.machine ?? UNKNOWN_MACHINE;
  made.client = client;
  // Looking at the machine takes the core a moment; nothing waits for it.
  client.profilesList().then(
    (list) => {
      if (profiles.getState().set(list)) made.views?.restyle();
    },
    (error: unknown) => notices.push("error", `The profiles could not be read: ${reason(error)}`),
  );

  const appearance = (): Appearance => {
    const view = config.getState().view;
    return view ? terminalAppearance(view.tokens, view.theme.scheme) : DEFAULT_APPEARANCE;
  };
  // The look is worked out once for each configuration, not once for each view.
  let look = appearance();
  // A terminal sizes its cells and draws its glyphs once: the font is there first.
  await loadTerminalFonts(look);
  startStep("fonts");

  const area = new TabAreaController({
    layout,
    config: () => config.getState().view,
    apply: (intent) => client.layoutApply(intent),
    defaults: DEFAULT_TOKENS,
  });

  /** The context of the palette on show, if one is: it takes every key. */
  const overlayContext = (): ReadonlySet<string> | undefined => {
    if (palette.getState().shown) return PALETTE_CONTEXT;
    if (connect.getState().shown) return CONNECT_CONTEXT;
    return undefined;
  };

  /** The contexts that hold where a key was pressed, with those the plugins set. */
  const withPlugins = (contexts: ReadonlySet<string>): ReadonlySet<string> => {
    const set = plugins.getState().contexts;
    return set.size === 0 ? contexts : new Set([...contexts, ...set]);
  };

  const platform = currentPlatform();
  // Running a command needs the views, and the views need the shortcuts.
  const shortcuts = new Shortcuts({ platform, layout: keyLayout }, (command, args) =>
    commands.run(command, args),
  );
  shortcuts.setBindings(configView.keymap.bindings);

  // The cursor's effects are drawn over the whole window, so that a trail can go from
  // one pane to another; the page gives them their element when it is rendered.
  const lessMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const cursorFx = new CursorFx((container) => host.createCursorEffects(container), lessMotion);
  // The plugins' pictures and shaders are fetched as they are needed.
  assets.connect((hashes) => client.uiAssets(hashes));
  // The effects the configuration names, with the plugins' shaders among them: a
  // shader not fetched yet is fetched, and the effects are set up again when it is
  // there, unless something else was chosen meanwhile.
  const configureCursorFx = (): void => {
    const view = config.getState().view;
    const shown = plugins.getState().shown;
    cursorFx.configure(cursorFxSettings(view, shown));
    const wanted = cursorFxShaders(view, shown).filter((hash) => assets.peek(hash) === undefined);
    if (wanted.length === 0) return;
    void assets.load(wanted).then(() => {
      if (wanted.some((hash) => assets.peek(hash) !== undefined)) {
        cursorFx.configure(cursorFxSettings(config.getState().view, plugins.getState().shown));
      }
    });
  };
  configureCursorFx();
  cursorFx.onProblem((id, message) => {
    const named = plugins.getState().shown?.cursor_effects.find((effect) => effect.id === id);
    notices.push(
      "warning",
      `The cursor effect ${named?.title ?? id} cannot be drawn, so none is: ${sentence(message)}`,
    );
  });

  // What the plugins add comes as the core has it; nothing waits for it.
  let contributionsRevision = -1;
  // The newest revision the core has told of; one told of while a fetch is on its way
  // is fetched once that one is in.
  let contributionsAnnounced = -1;
  let fetchingContributions = false;
  const fetchContributions = (): void => {
    if (fetchingContributions) return;
    fetchingContributions = true;
    client.uiContributions().then(
      (shown) => {
        fetchingContributions = false;
        if (shown.revision >= contributionsRevision) {
          contributionsRevision = shown.revision;
          plugins.getState().set(shown);
          assets.retry();
          configureCursorFx();
        }
        if (contributionsAnnounced > contributionsRevision) fetchContributions();
      },
      (error: unknown) => {
        fetchingContributions = false;
        notices.push("error", `What plugins add could not be read: ${reason(error)}`);
      },
    );
  };
  fetchContributions();
  const onLessMotion = () => cursorFx.configure();
  lessMotion.addEventListener("change", onLessMotion);
  const sizeCursorFx = () =>
    cursorFx.resize(
      { width: window.innerWidth, height: window.innerHeight },
      window.devicePixelRatio,
    );
  sizeCursorFx();
  // A move to a display of another pixel ratio resizes the window's page too.
  window.addEventListener("resize", sizeCursorFx);

  const views = new PaneViews({
    shared: {
      vt,
      host,
      cursor: cursorFx,
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
    reports,
    config: () => config.getState().view,
    appearance: () => look,
    appearanceOf: (pane) => appearanceOf(pane),
    shortcut: (pane, event, alternateScreen) => {
      // While a palette is on show every key is the palette's, wherever the keyboard
      // happens to be: none reaches a program.
      const overlay = overlayContext();
      if (overlay) {
        shortcuts.handle(event, overlay);
        return true;
      }
      const contexts = new Set(["terminal"]);
      if (alternateScreen) contexts.add("alt-screen");
      if (layout.getState().floats[pane]) contexts.add("floating");
      return shortcuts.handle(event, withPlugins(contexts));
    },
    keyboard: (pane) => search.setKeyboard(pane, "terminal"),
    searched: (pane) => search.touched(pane),
  });
  made.views = views;
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
  const tabUi = createTabUiStore();
  const commands = new Commands({
    store: commandStore,
    client,
    views,
    palette,
    connect,
    search,
    mayRun: (id, args) => mayRun({ layout, config, views, prompts, client, notices }, id, args),
    focusBar: () => focusBar(),
    renameTab: (tab) => tabUi.getState().startRename(tab ?? selectActiveTab(layout.getState())),
    answerPrompt: (proceed, item) => prompts.answerCurrent(proceed, item),
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
        configureCursorFx();
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
      case "window_shown":
        shownAgain();
        break;
      case "confirm_requested":
        void confirmRequested({ prompts, views, commands }, notice);
        break;
      case "profiles_changed":
        // A profile's look may be another now.
        if (profiles.getState().set(notice)) views.restyle();
        break;
      case "notice":
        notices.push(notice.level, notice.message, notice.action);
        break;
      case "contributions_changed":
        contributionsAnnounced = Math.max(contributionsAnnounced, notice.revision);
        if (notice.revision > contributionsRevision) fetchContributions();
        break;
      case "plugin_frame":
      case "plugin_state":
        plugins.getState().follow(notice);
        break;
      case "view_event":
        deliverViewEvent(notice.view, notice.event, notice.payload);
        break;
      case "commands_changed":
        // Plugins came or went: the palette and the menus offer what there is now.
        client.commandsList().then(
          (list) => commandStore.getState().set(list),
          (error: unknown) => {
            notices.push("error", `The commands could not be read again: ${reason(error)}`);
          },
        );
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
    ask: (ask) => prompts.askPlugin(ask),
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
      // The new pane's text is the size its profile asks for: the one the intent names,
      // or, for a split, the divided pane's.
      const list = profiles.getState().list;
      const content = "content" in intent ? intent.content : undefined;
      const named =
        content && content.type === "terminal" && "profile" in content
          ? content.profile
          : undefined;
      const paneLook =
        named !== undefined && named !== null
          ? looks.of(look, findProfile(list, named), list?.looks)
          : intent.type === "split"
            ? views.get(intent.pane)
              ? appearanceOf(intent.pane)
              : look
            : looks.of(look, findProfile(list, list?.default), list?.looks);
      const fontSizePx = Math.min(
        Math.max(paneLook.fontSizePx + views.zoomPx, FONT_SIZE_MIN_PX),
        FONT_SIZE_MAX_PX,
      );
      const dpr = window.devicePixelRatio;
      const metrics = host.measureCells(rendererTheme(paneLook, fontSizePx), dpr);
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

  // While a palette is on show the keyboard is the palette's, and when the palettes
  // are put away the keyboard goes back to where the user was working: the terminal
  // of the focused pane, or that pane's search bar if they were typing there.
  const refocus = (): void => {
    if (overlayContext()) return;
    const pane = selectFocus(layout.getState());
    if (pane !== null && search.focusBar(pane)) return;
    views.focus();
  };
  let overlayShown = false;
  const followOverlay = (): void => {
    const shown = overlayContext() !== undefined;
    if (shown === overlayShown) return;
    overlayShown = shown;
    if (!shown) queueMicrotask(refocus);
  };
  palette.subscribe(followOverlay);
  connect.subscribe(followOverlay);
  let focus: PaneId | null = null;
  let active: string | null = null;
  // The window is called what its tab on show is called.
  const retitle = (): void => {
    const shell = made.shell;
    const title = shell && active !== null ? describeTabNow(shell, active).label : DEFAULT_TITLE;
    if (document.title !== title) document.title = title;
  };
  reports.subscribeAll((pane) => {
    if (active !== null && layout.getState().paneTabs[pane] === active) retitle();
  });
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
    if (now !== null && windowState.getState().focused) reports.attend(now);
    retitle();
    // The card of a pane in a tab that has just been shown is rendered after this.
    queueMicrotask(refocus);
  });
  titles.subscribe((pane) => {
    if (pane === focus) retitle();
  });
  // A window that comes into use shows its focused pane, which has then been seen.
  windowState.subscribe((state, before) => {
    if (state.focused && !before.focused && focus !== null) reports.attend(focus);
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
    let contexts = overlayContext() ?? NO_CONTEXT;
    if (contexts === NO_CONTEXT && where.closest(SEARCH_BAR)) {
      // A search bar is not its terminal: a key that copies from a terminal, say,
      // must be left to the field.
      const pane = where.closest<HTMLElement>("[data-pane]")?.dataset.pane;
      contexts = new Set(["search"]);
      if (pane !== undefined && layout.getState().floats[pane]) {
        contexts = new Set(["search", "floating"]);
      }
    }
    if (shortcuts.handle(event, withPlugins(contexts))) {
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
  startStep("layout");
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

  const unfollowStatus = followPluginStatus(plugins, status, config);
  setMotionSource({
    view: () => config.getState().view,
    shown: () => plugins.getState().shown,
  });

  const shell: ShellApp = {
    layout,
    config,
    commandStore,
    client,
    area,
    views,
    titles,
    reports,
    commands,
    prompts,
    notices,
    palette,
    connect,
    profiles,
    machine,
    status,
    plugins,
    visible,
    expandRow: (panel, path) => client.uiPanelExpand(panel, path),
    windowState,
    cardDrag,
    panelDrag,
    search,
    tabUi,
    cursorFx,
    stats: () => ({ ...host.stats(), wasmBytes: vt.memoryBytes }),
    dispose: () => {
      window.removeEventListener("focus", onWindowFocus);
      window.removeEventListener("keydown", onWindowKey, true);
      document.removeEventListener("visibilitychange", onVisibility);
      unfollowStatus();
      visible.dispose();
      setMotionSource(null);
      onNotice = () => undefined;
      onRequest = () => undefined;
      search.reset();
      turns.port1.close();
      views.dispose();
      lessMotion.removeEventListener("change", onLessMotion);
      window.removeEventListener("resize", sizeCursorFx);
      cursorFx.detach();
      host.dispose();
    },
  };
  made.shell = shell;
  // What the profiles and the configuration say changes what the window is called.
  profiles.subscribe(retitle);
  config.subscribe(retitle);
  retitle();
  return shell;
}

let app: Promise<ShellApp> | undefined;

/** The window's page. The first call starts it; every call gets the same one. */
export function shellApp(): Promise<ShellApp> {
  app ??= startShell(new TauriCarrier());
  return app;
}
