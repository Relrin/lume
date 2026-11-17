/**
 * Asking before something is closed that would end programs still running in it.
 *
 * Whether to ask is a setting. A pane or a tab is asked about here, before the command
 * that closes it is run; a window is asked about when the core says its close must be
 * confirmed, and is then closed with the answer attached. Only what the user closes is
 * asked about: a program that ends by itself closes its pane without a question.
 */

import {
  selectActiveTab,
  selectFocus,
  selectTabOrder,
  selectTabPanes,
  selectWindowPanes,
} from "../layout/selectors.ts";
import type { LayoutState } from "../layout/store.ts";
import type { GroupId, PaneId, TabId } from "../layout/types.ts";
import type { Closing } from "../terminal/prompts.ts";
import type { ShellApp } from "./boot.ts";
import { sentence } from "./notices.ts";

/** The parts of the page asking about a close needs. */
export type ClosingApp = Pick<ShellApp, "layout" | "config" | "views" | "prompts" | "client"> &
  Partial<Pick<ShellApp, "notices">>;

/** The commands that close panes the user may still be using. */
const PANE_CLOSE = "lume.pane.close";
const TAB_CLOSE = "lume.tab.close";
const TABS_CLOSE_LEFT = "lume.tab.close-left";
const TABS_CLOSE_RIGHT = "lume.tab.close-right";
const GROUP_CLOSE = "lume.group.close";
const GROUP_MOVE = "lume.group.move-to-workspace";

/** The group of the tab on show, if it is in one. */
function groupOnShow(state: LayoutState): GroupId | undefined {
  const tab = selectActiveTab(state);
  return (tab === null ? undefined : state.tabs[tab]?.group) ?? undefined;
}

function named(args: unknown, key: string): string | undefined {
  if (typeof args !== "object" || args === null || !(key in args)) return undefined;
  const value = (args as Readonly<Record<string, unknown>>)[key];
  return typeof value === "string" ? value : undefined;
}

/**
 * The panes a command would close, given what it is run with: the pane or tab it
 * names, or the one that has the keyboard. Nothing for a command that closes none.
 */
export function panesClosedBy(
  state: LayoutState,
  id: string,
  args: unknown,
): { readonly what: Closing["what"]; readonly panes: readonly PaneId[] } | undefined {
  if (!state.ready) return undefined;
  if (id === PANE_CLOSE) {
    const pane = (named(args, "pane") as PaneId | undefined) ?? selectFocus(state);
    return pane === null ? undefined : { what: "pane", panes: [pane] };
  }
  if (id === TAB_CLOSE) {
    const tab = (named(args, "tab") as TabId | undefined) ?? selectActiveTab(state);
    // A locked tab is not closed.
    if (tab === null || state.tabs[tab]?.locked === true) return undefined;
    return { what: "tab", panes: selectTabPanes(state, tab) };
  }
  if (id === TABS_CLOSE_LEFT || id === TABS_CLOSE_RIGHT) {
    const tab = (named(args, "tab") as TabId | undefined) ?? selectActiveTab(state);
    const window = tab === null ? undefined : state.tabs[tab]?.window;
    if (tab === null || window === undefined) return undefined;
    const order = selectTabOrder(state, window);
    const at = order.indexOf(tab);
    const side = id === TABS_CLOSE_LEFT ? order.slice(0, at) : order.slice(at + 1);
    const closed = side.filter((other) => state.tabs[other]?.locked !== true);
    return closed.length === 0
      ? undefined
      : { what: "tabs", panes: closed.flatMap((other) => selectTabPanes(state, other)) };
  }
  if (id === GROUP_CLOSE || id === GROUP_MOVE) {
    const group = (named(args, "group") as GroupId | undefined) ?? groupOnShow(state);
    if (group === undefined) return undefined;
    const tab = selectActiveTab(state);
    const window = (tab === null ? undefined : state.tabs[tab]?.window) ?? state.window;
    const tabs = selectTabOrder(state, window ?? undefined).filter(
      (other) =>
        state.tabs[other]?.group === group &&
        // A locked tab stays when its group is closed; it goes when its group moves.
        (id === GROUP_MOVE || state.tabs[other]?.locked !== true),
    );
    return tabs.length === 0
      ? undefined
      : {
          what: id === GROUP_MOVE ? "moved-group" : "group",
          panes: tabs.flatMap((other) => selectTabPanes(state, other)),
        };
  }
  return undefined;
}

/** The title of the question about a close. */
export function closingTitle(closing: Closing): string {
  switch (closing.what) {
    case "tabs":
      return "Close these tabs?";
    case "moved-group":
      return "Move this group?";
    default:
      return `Close this ${closing.what}?`;
  }
}

/** What the question about a close says, under its title. */
export function closingText(closing: Closing): string {
  const { what, running } = closing;
  const one = running === 1;
  const has = what === "pane" ? "It has" : what === "tabs" ? "Their panes have" : "Its panes have";
  const programs = one
    ? "a program that is still running"
    : `${running} programs that are still running`;
  const doing =
    what === "moved-group"
      ? "Moving the group to another workspace"
      : `Closing the ${what === "tabs" ? "tabs" : what}`;
  // The count is of panes: a pane with a command running counts once.
  return `${has} ${programs}. ${doing} ends ${one ? "it" : "them"}.`;
}

/**
 * How many of some panes run something that closing them would end. The core knows
 * best: which program has each terminal, and whether a shell is running a command. If
 * it cannot be asked, a pane whose program has not ended counts.
 */
async function running(app: ClosingApp, panes: readonly PaneId[]): Promise<number> {
  if (panes.length === 0) return 0;
  try {
    return (await app.client.panesBusy(panes)).length;
  } catch {
    return panes.filter((pane) => app.views.get(pane)?.ended === false).length;
  }
}

/**
 * Whether a command may be run now. It may, unless closing a pane asks first, the
 * command would close a pane whose program is still running, and the user says no.
 */
export async function mayRun(app: ClosingApp, id: string, args: unknown): Promise<boolean> {
  const view = app.config.getState().view;
  if (view?.config.terminal["confirm-close"] !== "always") return true;
  const closed = panesClosedBy(app.layout.getState(), id, args);
  if (!closed) return true;
  const live = await running(app, closed.panes);
  if (live === 0) return true;
  const agreed = await app.prompts.confirmClose({ what: closed.what, running: live });
  // The keyboard goes back to where the user was working when nothing is closed.
  if (!agreed) app.views.focus();
  return agreed;
}

/**
 * Answers the core's word that this window was asked to close and must ask first:
 * asks, if a program is still running in the window, and closes the window on a yes.
 */
export async function closeRequested(app: ClosingApp): Promise<void> {
  const state = app.layout.getState();
  const live = state.ready ? await running(app, selectWindowPanes(state)) : 0;
  if (live > 0 && !(await app.prompts.confirmClose({ what: "window", running: live }))) {
    app.views.focus();
    return;
  }
  try {
    await app.client.windowControl("close", true);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    app.notices?.push("error", sentence(message));
  }
}
