/**
 * Running a command from the page: from a key, from a button, or because the core
 * passed one on.
 *
 * A command that changes the layout, the configuration or the application is carried
 * out in the core, and the page only asks. A command that acts on what only the page
 * has is carried out here: a terminal's selection and scrollback, and the size of the
 * window's text.
 */

import type { CoreClient } from "../core/client.ts";
import type { PaneId } from "../layout/types.ts";
import type { PaneViews } from "../panes/views.ts";
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
  /** Tells the user that a command they ran could not be carried out. */
  failed(message: string): void;
}

function paneOf(args: unknown): PaneId | undefined {
  if (typeof args !== "object" || args === null || !("pane" in args)) return undefined;
  return typeof args.pane === "string" ? args.pane : undefined;
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
    client.commandRun(id, args).catch((error: unknown) => {
      failed(error instanceof Error ? error.message : String(error));
    });
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
    const { views } = this.#services;
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
      default:
        throw new CommandError("not_found", `the page has no command called ${id}`);
    }
  }
}
