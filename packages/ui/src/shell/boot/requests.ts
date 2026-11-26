/**
 * What the core may ask of the window: what a pane shows, how big a new pane would be,
 * a question for the user. The window answers once it has gone live.
 */

import type { RenderHost } from "@lume/renderer";

import type { Commands } from "../../commands/run.ts";
import type { CoreClient } from "../../core/client.ts";
import type { TabAreaController } from "../../layout/controller.ts";
import { estimatePane, gridFor } from "../../layout/estimate.ts";
import { panesOf } from "../../layout/selectors.ts";
import type { LayoutStore } from "../../layout/store.ts";
import type { Intent, PaneId } from "../../layout/types.ts";
import type { PaneLooks } from "../../panes/looks.ts";
import type { PaneViews } from "../../panes/views.ts";
import { findProfile, type ProfilesStore } from "../../profiles/store.ts";
import type { PromptStore } from "../../terminal/prompts.ts";
import { answerRequest, type PaneRef, type RemoteTarget } from "../../terminal/remote.ts";
import type { TerminalView } from "../../terminal/view.ts";
import { rendererTheme, type Appearance } from "../../theme.ts";
import { CommandError } from "../../transport/types.ts";
import { FONT_SIZE_MAX_PX, FONT_SIZE_MIN_PX } from "../../tuning.ts";

export interface RequestDeps {
  readonly host: RenderHost;
  readonly layout: LayoutStore;
  readonly profiles: ProfilesStore;
  readonly looks: PaneLooks;
  readonly area: TabAreaController;
  readonly views: PaneViews;
  readonly commands: Commands;
  readonly prompts: PromptStore;
  /** The look of the window's terminals. */
  readonly look: () => Appearance;
  /** The look of a pane's terminal. */
  readonly appearanceOf: (pane: PaneId) => Appearance;
  /** The keyboard goes back to where it was, if nothing else in the window has it. */
  readonly onWindowFocus: () => void;
}

/** What answers the core's requests about this window. */
export function remoteTarget(deps: RequestDeps): RemoteTarget {
  const { host, layout, profiles, looks, area, views, commands, prompts, look } = deps;

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

  return {
    ask: (ask) => prompts.askPlugin(ask),
    signIn: async (signIn) => {
      const answered = await prompts.signIn(signIn);
      // The dialog gives the keyboard back to what had it, if that is still there.
      setTimeout(deps.onWindowFocus);
      return answered;
    },
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
          ? looks.of(look(), findProfile(list, named), list?.looks)
          : intent.type === "split"
            ? views.get(intent.pane)
              ? deps.appearanceOf(intent.pane)
              : look()
            : looks.of(look(), findProfile(list, list?.default), list?.looks);
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
}

/** What answers each request the core sends, for `CoreGate.goLive`. */
export function answering(
  target: RemoteTarget,
  client: Pick<CoreClient, "answer">,
): (message: unknown) => void {
  return (message) => {
    void answerRequest(target, message).then((answer) => {
      if (!answer) return;
      client.answer(answer.id, answer.result).catch((error: unknown) => {
        console.error("an answer to the core was not delivered", error);
      });
    });
  };
}
