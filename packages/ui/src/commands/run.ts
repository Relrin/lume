/**
 * Running a command from the page: from a key, from a button, or because the core
 * passed one on.
 *
 * A command that changes the layout, the configuration or the application is carried
 * out in the core, and the page only asks. A command that acts on what only the page
 * has is carried out here: a terminal's selection and scrollback, searching it, the
 * size of the window's text, and the palettes.
 */

import type { ConnectStore } from "../connect/store.ts";
import type { CoreClient } from "../core/client.ts";
import type { PaneId } from "../layout/types.ts";
import type { PaletteStore } from "../palette/store.ts";
import type { PaneViews } from "../panes/views.ts";
import type { SearchController } from "../search/controller.ts";
import type { TerminalAction } from "../terminal/keymap.ts";
import type { TerminalView } from "../terminal/view.ts";
import { CommandError } from "../transport/types.ts";
import { FONT_ZOOM_STEP_PX } from "../tuning.ts";
import type { CommandStore } from "./store.ts";

/** The commands that act on a terminal, and what each does to it. */
const TERMINAL_ACTIONS: Readonly<Record<string, TerminalAction>> = {
  "lume.terminal.copy": "copy",
  "lume.terminal.paste": "paste",
  "lume.terminal.select-all": "select-all",
  "lume.terminal.clear": "clear",
  "lume.terminal.scroll-page-up": "scroll-page-up",
  "lume.terminal.scroll-page-down": "scroll-page-down",
  "lume.terminal.scroll-top": "scroll-top",
  "lume.terminal.scroll-bottom": "scroll-bottom",
};

/** What running commands needs from its surroundings. */
export interface CommandServices {
  readonly store: CommandStore;
  readonly client: CoreClient;
  readonly views: PaneViews;
  readonly palette: PaletteStore;
  /** The palette that opens a terminal from a profile. */
  readonly connect: ConnectStore;
  readonly search: SearchController;
  /** Asks the user before a command that closes something, if they want to be asked. */
  mayRun(id: string, args: unknown): Promise<boolean>;
  /** Moves the keyboard to the window's bar. */
  focusBar(): void;
  /** Opens the field that renames a tab in place: the one named, or the tab on show. */
  renameTab(tab: string | null): void;
  /** Opens the field that renames a pane in place: the one named, or the one in use. */
  renamePane(pane: PaneId | null): void;
  /** Shows the dialog that opens a folder as a workspace. */
  openFolder(): void;
  /**
   * Answers the question on show as its buttons would: going ahead (with the item of a
   * choice), or not. Says whether there was one.
   */
  answerPrompt(proceed: boolean, item?: string): boolean;
  /** Tells the user that a command they ran could not be carried out. */
  failed(message: string): void;
}

function paneOf(args: unknown): PaneId | undefined {
  if (typeof args !== "object" || args === null || !("pane" in args)) return undefined;
  return typeof args.pane === "string" ? args.pane : undefined;
}

function tabOf(args: unknown): string | undefined {
  if (typeof args !== "object" || args === null || !("tab" in args)) return undefined;
  return typeof args.tab === "string" ? args.tab : undefined;
}

function queryOf(args: unknown): string | undefined {
  if (typeof args !== "object" || args === null || !("query" in args)) return undefined;
  return typeof args.query === "string" ? args.query : undefined;
}

/** Runs commands in this window. */
export class Commands {
  readonly #services: CommandServices;

  constructor(services: CommandServices) {
    this.#services = services;
  }

  /**
   * Runs a command for the user. It never throws: a command that fails is told to the
   * user, and a command there is no such thing as is passed over, since a key may be
   * bound to a command this version does not have.
   */
  run(id: string, args: unknown = null): void {
    const { store, client, failed } = this.#services;
    const spec = store.getState().find(id);
    if (!spec) return;
    if (spec.runs === "page") {
      try {
        this.runInPage(id, args);
      } catch (error) {
        failed(error instanceof Error ? error.message : String(error));
      }
      return;
    }
    const report = (error: unknown) =>
      failed(error instanceof Error ? error.message : String(error));
    // The user may want to be asked before a pane with a program in it is closed.
    this.#services
      .mayRun(id, args)
      .then((may) => (may ? client.commandRun(id, args) : undefined))
      .catch(report);
  }

  /** The pane a command acts on: the one its arguments name, or the focused one. */
  #pane(args: unknown): PaneId {
    const { views } = this.#services;
    const named = paneOf(args);
    const pane = named ?? views.focused()?.pane;
    if (pane === undefined || !views.get(pane)) {
      throw new CommandError(
        "not_shown",
        named === undefined ? "no pane has the keyboard" : `there is no pane ${named}`,
      );
    }
    return pane;
  }

  /** The view a command acts on: the pane its arguments name, or the focused one. */
  #view(args: unknown): TerminalView {
    const { views } = this.#services;
    const named = paneOf(args);
    const view = named === undefined ? views.focused()?.view : views.get(named);
    if (!view) {
      throw new CommandError(
        "not_shown",
        named === undefined ? "no pane has the keyboard" : `there is no pane ${named}`,
      );
    }
    return view;
  }

  /**
   * Carries out a command that runs in the page. Throws a {@link CommandError} when it
   * cannot be: there is no such command here, or nothing for it to act on.
   */
  runInPage(id: string, args: unknown = null): void {
    const { views, palette, connect, search } = this.#services;
    const action = TERMINAL_ACTIONS[id];
    if (action !== undefined) {
      this.#view(args).perform(action);
      return;
    }
    switch (id) {
      case "lume.view.zoom-in":
        views.setZoom(views.zoomPx + FONT_ZOOM_STEP_PX);
        return;
      case "lume.view.zoom-out":
        views.setZoom(views.zoomPx - FONT_ZOOM_STEP_PX);
        return;
      case "lume.view.zoom-reset":
        views.setZoom(0);
        return;
      case "lume.palette.open":
        connect.getState().close();
        palette.getState().toggle();
        return;
      case "lume.connect.open":
        palette.getState().close();
        connect.getState().toggle();
        return;
      case "lume.session.detached":
        palette.getState().close();
        connect.getState().open("detached");
        return;
      case "lume.search.open": {
        const pane = this.#pane(args);
        search.open(pane, queryOf(args));
        // A bar that was on show already is given the keyboard; a new one takes it.
        search.focusBar(pane, true);
        return;
      }
      case "lume.search.next":
      case "lume.search.previous": {
        const pane = this.#pane(args);
        if (!search.of(pane).open) search.open(pane);
        search.step(pane, id === "lume.search.next" ? "older" : "newer");
        return;
      }
      case "lume.search.close": {
        const pane = this.#pane(args);
        search.close(pane);
        if (views.focused()?.pane === pane) views.focus();
        return;
      }
      case "lume.window.focus-bar":
        this.#services.focusBar();
        return;
      case "lume.tab.rename-start":
        this.#services.renameTab(tabOf(args) ?? null);
        return;
      case "lume.pane.rename-start":
        this.#services.renamePane(paneOf(args) ?? null);
        return;
      case "lume.workspace.open-folder":
        palette.getState().close();
        connect.getState().close();
        this.#services.openFolder();
        return;
      case "lume.window.answer": {
        const fields =
          typeof args === "object" && args !== null ? (args as Record<string, unknown>) : {};
        const proceed = fields["proceed"] === true;
        const item = typeof fields["item"] === "string" ? fields["item"] : undefined;
        if (!this.#services.answerPrompt(proceed, item)) {
          throw new CommandError("not_found", "there is no question on show");
        }
        return;
      }
      default:
        throw new CommandError("not_found", `the page has no command called ${id}`);
    }
  }
}
