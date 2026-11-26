/**
 * The keyboard and the window's title follow the layout's focus.
 *
 * Everything here listens to stores; it is set up before the layout's first copy is
 * taken in, so that the copy is followed like every change after it.
 */

import type { ConfigStore } from "../../config/store.ts";
import type { ConnectStore } from "../../connect/store.ts";
import type { PaneReports } from "../../events/store.ts";
import { selectActiveTab, selectFocus } from "../../layout/selectors.ts";
import type { LayoutState, LayoutStore } from "../../layout/store.ts";
import type { PaneId } from "../../layout/types.ts";
import type { PaletteStore } from "../../palette/store.ts";
import type { PaneTitles } from "../../panes/titles.ts";
import type { PaneViews } from "../../panes/views.ts";
import { focusPaneFrame, onPaneFrameReady } from "../../plugins/frame/registry.ts";
import type { ProfilesStore } from "../../profiles/store.ts";
import type { SearchController } from "../../search/controller.ts";
import type { PromptStore } from "../../terminal/prompts.ts";
import type { WindowStateStore } from "../../window/state.ts";
import type { ShellApp } from "./app.ts";
import { describeTabNow } from "../describing.ts";
import type { Disposer } from "./disposer.ts";
import { NO_CONTEXT, type Input } from "./input.ts";
import type { Late } from "./late.ts";

/** A pane's search bar, as an element is asked whether it is in one. */
const SEARCH_BAR = "[data-search-bar]";

/**
 * The parts of the window that keep the keys pressed in them: a terminal offers its
 * own keys to the bindings, and a dialog or a menu takes what is typed in it.
 */
const KEEPS_KEYS = ".terminal, [role=alertdialog], [role=menu]";

/** The title the window has when no program has given the focused pane one. */
const DEFAULT_TITLE = "Lume";

export interface FocusDeps {
  readonly layout: LayoutStore;
  readonly config: ConfigStore;
  readonly palette: PaletteStore;
  readonly connect: ConnectStore;
  readonly profiles: ProfilesStore;
  readonly windowState: WindowStateStore;
  readonly reports: PaneReports;
  readonly titles: PaneTitles;
  readonly prompts: PromptStore;
  readonly views: PaneViews;
  readonly search: SearchController;
  readonly input: Input;
  /** The window's app, which the title is worked out from. */
  readonly app: Late<ShellApp>;
  readonly disposer: Disposer;
}

export interface Focus {
  /** Gives the keyboard back to where the user was working. */
  refocus(): void;
  /** The window came into use: the keyboard comes back if nothing else has it. */
  onWindowFocus(): void;
  /** Calls the window what its tab on show is called. */
  retitle(): void;
}

export function followFocus(deps: FocusDeps): Focus {
  const { layout, config, palette, connect, profiles, windowState, reports, titles } = deps;
  const { prompts, views, search, input, app, disposer } = deps;

  // While a palette is on show the keyboard is the palette's, and when the palettes
  // are put away the keyboard goes back to where the user was working: the terminal
  // of the focused pane, or that pane's search bar if they were typing there.
  const refocus = (): void => {
    if (input.overlayContext()) return;
    // What is typed there is for nothing else.
    if (prompts.current()?.kind === "sign-in") return;
    const pane = selectFocus(layout.getState());
    if (pane !== null && search.focusBar(pane)) return;
    // A pane that shows a plugin's view has no terminal: its frame takes the keyboard.
    if (pane !== null && views.get(pane) === undefined && focusPaneFrame(pane)) return;
    views.focus();
  };
  // A view's frame that has just become ready takes the keyboard if its pane has it.
  onPaneFrameReady((pane) => {
    if (selectFocus(layout.getState()) === pane) queueMicrotask(refocus);
  });
  disposer.add(() => onPaneFrameReady(null));

  let overlayShown = false;
  const followOverlay = (): void => {
    const shown = input.overlayContext() !== undefined;
    if (shown === overlayShown) return;
    overlayShown = shown;
    if (!shown) queueMicrotask(refocus);
  };
  disposer.add(palette.subscribe(followOverlay));
  disposer.add(connect.subscribe(followOverlay));

  let focus: PaneId | null = null;
  let active: string | null = null;
  // The window is called what its tab on show is called.
  const retitle = (): void => {
    const title = active !== null ? describeTabNow(app.get(), active).label : DEFAULT_TITLE;
    if (document.title !== title) document.title = title;
  };
  disposer.add(
    reports.subscribeAll((pane) => {
      if (active !== null && layout.getState().paneTabs[pane] === active) retitle();
    }),
  );
  let epoch: number | undefined;
  const followLayout = (state: LayoutState): void => {
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
  };
  disposer.add(layout.subscribe(followLayout));
  disposer.add(
    titles.subscribe((pane) => {
      if (pane === focus) retitle();
    }),
  );
  // A window that comes into use shows its focused pane, which has then been seen.
  disposer.add(
    windowState.subscribe((state, before) => {
      if (state.focused && !before.focused && focus !== null) reports.attend(focus);
    }),
  );
  // What the profiles and the configuration say changes what the window is called.
  disposer.add(profiles.subscribe(retitle));
  disposer.add(config.subscribe(retitle));

  const onWindowFocus = (): void => {
    // The keyboard comes back to the terminal unless something else in the window
    // was using it: a dialog, or a field being typed in.
    const using = document.activeElement;
    if (using === null || using === document.body) refocus();
  };
  window.addEventListener("focus", onWindowFocus);
  disposer.add(() => window.removeEventListener("focus", onWindowFocus));

  // Keys pressed while the keyboard is with the window's own controls, not with a
  // terminal, are offered to the bindings that hold everywhere, and to those that
  // hold where the keyboard is.
  const onWindowKey = (event: KeyboardEvent): void => {
    const where = event.target;
    if (!(where instanceof Element)) return;
    if (where.closest(KEEPS_KEYS)) return;
    let contexts = input.overlayContext() ?? NO_CONTEXT;
    if (contexts === NO_CONTEXT && where.closest(SEARCH_BAR)) {
      // A search bar is not its terminal: a key that copies from a terminal, say,
      // must be left to the field.
      const pane = where.closest<HTMLElement>("[data-pane]")?.dataset.pane;
      contexts = new Set(["search"]);
      if (pane !== undefined && layout.getState().floats[pane]) {
        contexts = new Set(["search", "floating"]);
      }
    }
    if (input.shortcuts.handle(event, input.withPlugins(contexts))) {
      event.preventDefault();
      event.stopPropagation();
    }
  };
  window.addEventListener("keydown", onWindowKey, true);
  disposer.add(() => window.removeEventListener("keydown", onWindowKey, true));

  return { refocus, onWindowFocus, retitle };
}
