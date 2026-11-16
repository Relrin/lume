/**
 * The bar along the top of the window.
 *
 * It has three places for status items: at its left, in its middle and at its right.
 * Around them stand the bar's own parts: the application's menu and the workspace's
 * name at the left end, the window's tabs in the middle unless they are listed in a panel at the window's side,
 * and the window's buttons at the right end when the page draws the title bar.
 *
 * A window without a title bar of the system's is moved by this bar: a press on any
 * part of it that is not a control drags the window, and a double press makes the
 * window fill the screen or puts it back.
 */

import { useLayoutEffect, useRef } from "react";
import { useStore } from "zustand";

import { hasTabListPanel, selectPanels } from "../layout/selectors.ts";
import type { Slot } from "../layout/types.ts";
import { StatusSlotItems } from "../status/StatusItem.tsx";
import { TRAFFIC_LIGHTS_INSET_PX } from "../tuning.ts";
import { AppMenu } from "./AppMenu.tsx";
import { useShell } from "./context.ts";
import { TabStrip } from "./TabStrip.tsx";
import { WindowControls } from "./WindowControls.tsx";
import { WorkspaceSwitcher } from "./WorkspaceSwitcher.tsx";

/** The parts of the bar a press is meant for, and so does not move the window. */
const CONTROLS = "button, input, textarea, select, a, [role=menu], .tab";

/** The top bar. */
export function TopBar() {
  const app = useShell();
  const bar = useRef<HTMLElement>(null);
  const custom = useStore(app.config, (state) => state.view?.title_bar === "custom");
  const mac = useStore(app.config, (state) => state.view?.platform === "macos");
  const tabsAtSide = useStore(app.layout, (state) => state.ready && hasTabListPanel(state));
  const panels = useStore(app.layout, (state) => selectPanels(state, "top"));
  const has = (slot: Slot) =>
    panels.some((panel) => panel.content.type === "slot" && panel.content.slot === slot);

  // The system's own window buttons lie over the left end of the bar on a Mac.
  const inset = custom && mac;
  useLayoutEffect(() => {
    const element = bar.current;
    if (element) element.style.paddingLeft = inset ? `${TRAFFIC_LIGHTS_INSET_PX}px` : "";
  }, [inset]);

  return (
    <header
      ref={bar}
      className="top-bar"
      data-title-bar={custom ? "custom" : "system"}
      onMouseDown={(event) => {
        if (event.button !== 0 || (event.target as Element).closest(CONTROLS)) return;
        // The keyboard stays with the terminal when the bar is pressed.
        event.preventDefault();
        if (!custom) return;
        const asked =
          event.detail === 2 ? app.client.windowControl("maximise") : app.client.windowDrag();
        asked.catch(() => undefined);
      }}
    >
      <div className="top-left">
        <AppMenu />
        <WorkspaceSwitcher />
        {has("topbar.left") && <StatusSlotItems store={app.status} slot="topbar.left" />}
      </div>
      {!tabsAtSide && <span className="top-divider" />}
      <div className="top-centre">
        {!tabsAtSide && <TabStrip />}
        {has("topbar.center") && <StatusSlotItems store={app.status} slot="topbar.center" />}
      </div>
      <div className="top-right">
        {has("topbar.right") && <StatusSlotItems store={app.status} slot="topbar.right" />}
        {custom && !mac && <WindowControls />}
      </div>
    </header>
  );
}
