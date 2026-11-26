/**
 * What the page takes from the core as it starts: its first copies, the lists that take
 * the core a moment, and which of the plugins' things are on screen.
 */

import { applyConfig } from "../../config/apply.ts";
import type { CoreClient } from "../../core/client.ts";
import type { CommandSpec, ConfigView, LayoutView, Machine } from "../../core/types.ts";
import type { PaneViews } from "../../panes/views.ts";
import { VisibleItems } from "../../plugins/visible.ts";
import type { NoticeStore } from "../notices.ts";
import type { Disposer } from "./disposer.ts";
import type { Late } from "./late.ts";
import { reason } from "./reason.ts";
import type { Stores } from "./stores.ts";

/** A machine the page was told nothing about. */
const UNKNOWN_MACHINE: Machine = { name: null, home: null };

/** Takes in the first copies of the configuration and the commands; gives the machine. */
export function takeCopies(
  { config, commandStore, hostLink }: Stores,
  layoutView: LayoutView,
  configView: ConfigView,
  commands: readonly CommandSpec[],
): Machine {
  config.getState().set(configView);
  commandStore.getState().set(commands);
  applyConfig(configView);
  hostLink.getState().set(layoutView.host?.connected ?? true);
  return layoutView.machine ?? UNKNOWN_MACHINE;
}

/**
 * Asks for what takes the core a moment, looking at the machine; nothing waits for it.
 * A profile's look may change a terminal's: profiles that come before the views are
 * made style them as soon as they are.
 */
export function askSlowly(
  client: Pick<CoreClient, "profilesList" | "rexHosts">,
  { profiles, rex }: Stores,
  views: Late<PaneViews>,
  notices: Pick<NoticeStore, "push">,
): void {
  client.profilesList().then(
    (list) => {
      if (profiles.getState().set(list)) views.whenSet((made) => made.restyle());
    },
    (error: unknown) => notices.push("error", `The profiles could not be read: ${reason(error)}`),
  );
  client.rexHosts().then(
    (list) => void rex.getState().set(list),
    (error: unknown) =>
      notices.push("error", `The Rex servers could not be read: ${reason(error)}`),
  );
}

/**
 * Which of the plugins' things the window has on screen, as the core is told; none
 * while the window is out of sight.
 */
export function watchVisible(client: CoreClient, disposer: Disposer): VisibleItems {
  const visible = disposer.use(
    new VisibleItems((items) => void client.uiVisible(items).catch(() => undefined)),
  );
  const onVisibility = () => visible.setShown(document.visibilityState !== "hidden");
  document.addEventListener("visibilitychange", onVisibility);
  disposer.add(() => document.removeEventListener("visibilitychange", onVisibility));
  onVisibility();
  return visible;
}
