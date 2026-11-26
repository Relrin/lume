/**
 * Following the core: what each of its notices changes in the window.
 *
 * Every store and controller a notice touches exists before the page goes live, so a
 * notice kept while the page was starting is followed like any other.
 */

import type { CommandStore } from "../../commands/store.ts";
import { describeProblem } from "../../config/apply.ts";
import type { ConfigStore } from "../../config/store.ts";
import type { CoreClient } from "../../core/client.ts";
import type { ConfigView, Notice } from "../../core/types.ts";
import type { LayoutStore } from "../../layout/store.ts";
import { deliverViewEvent, reloadPluginFrames } from "../../plugins/frame/registry.ts";
import type { PluginsStore } from "../../plugins/store.ts";
import type { ProfilesStore } from "../../profiles/store.ts";
import type { RexStore } from "../../rex/store.ts";
import { shownAgain } from "../../start-marks.ts";
import type { PromptStore } from "../../terminal/prompts.ts";
import type { WindowStateStore } from "../../window/state.ts";
import type { HostLinkStore } from "../host-link.ts";
import type { NoticeStore } from "../notices.ts";
import type { Contributions } from "./contributions.ts";
import { reason } from "./reason.ts";

export type ConfirmNotice = Extract<Notice, { type: "confirm_requested" }>;

export interface FollowDeps {
  readonly layout: LayoutStore;
  readonly config: ConfigStore;
  readonly commandStore: CommandStore;
  readonly windowState: WindowStateStore;
  readonly profiles: ProfilesStore;
  readonly rex: RexStore;
  readonly hostLink: HostLinkStore;
  readonly plugins: PluginsStore;
  readonly client: Pick<CoreClient, "layoutView" | "commandsList">;
  readonly notices: Pick<NoticeStore, "push">;
  readonly prompts: Pick<PromptStore, "withdraw">;
  readonly contributions: Pick<Contributions, "announced">;
  /** The configuration in use is another now: the window takes on what it says. */
  configChanged(view: ConfigView): void;
  /** A profile's look may be another now. */
  restyle(): void;
  /** The user asked to close the window. */
  closeRequested(): void;
  /** The core asks the user to confirm a command before it is run. */
  confirmRequested(notice: ConfirmNotice): void;
}

/** What follows the core's notices, for `CoreGate.goLive`. */
export function followCore(deps: FollowDeps): (notice: Notice) => void {
  const { layout, config, commandStore, windowState, profiles, rex, hostLink, plugins } = deps;
  const { client, notices, prompts, contributions } = deps;

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

  return (notice) => {
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
        if (config.getState().set(notice.view)) deps.configChanged(notice.view);
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
        deps.closeRequested();
        break;
      case "window_shown":
        shownAgain();
        break;
      case "confirm_requested":
        deps.confirmRequested(notice);
        break;
      case "profiles_changed":
        if (profiles.getState().set(notice)) deps.restyle();
        break;
      case "rex_hosts_changed":
        rex.getState().set(notice);
        break;
      case "notice":
        notices.push(notice.level, notice.message, notice.action);
        break;
      case "sign_in_withdrawn":
        prompts.withdraw(notice.ask);
        break;
      case "host_link":
        hostLink.getState().set(notice.connected);
        break;
      case "contributions_changed":
        contributions.announced(notice.revision);
        break;
      case "plugin_frame":
      case "plugin_state":
        plugins.getState().follow(notice);
        break;
      case "view_event":
        deliverViewEvent(notice.view, notice.event, notice.payload);
        break;
      case "plugin_view_reload":
        reloadPluginFrames(notice.plugin);
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
}
