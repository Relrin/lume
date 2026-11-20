/**
 * One panel docked at a side of the window: a small header and whatever the panel
 * shows under it.
 *
 * The header names the panel, folds its side of the window away, and is what the panel
 * is dragged by to another side. The list of tabs goes to the left or the right only,
 * and where it is belongs to the settings, so dropping it at the other side changes
 * the setting; any other panel is simply moved.
 *
 * A plugin's list panel shows the rows the plugin's code gives, and can be closed. A
 * panel of a plugin that is not there, or cannot be used, says whose it is.
 */

import { lazy, Suspense, useRef } from "react";
import { useStore } from "zustand";

import type { PanelDef } from "../core/types.ts";
import { panelDropArea, type SideArea } from "../drag/panels.ts";
import { SIDE_AREAS, selectPanels, TAB_LIST_PANEL } from "../layout/selectors.ts";
import type { Panel } from "../layout/types.ts";
import { useSlotMotion } from "../motion/slots.ts";
import { DevBadge } from "../plugins/DevBadge.tsx";
import { PluginFrame } from "../plugins/frame/PluginFrame.tsx";
import { PluginIcon } from "../plugins/PluginIcon.tsx";
import { DRAG_START_PX } from "../tuning.ts";
import { useShell } from "./context.ts";
import { useKeyHint } from "./hints.ts";
import { CloseIcon, CollapseIcon } from "./icons.tsx";
import { PluginViewPlaceholder } from "./PluginViewPlaceholder.tsx";
import { TabListPanel } from "./TabListPanel.tsx";

// A plugin's list panel is a part of the page a window without one never loads.
const ListPanel = lazy(() =>
  import("./ListPanel.tsx").then((loaded) => ({ default: loaded.ListPanel })),
);

/** The sides the list of tabs may be docked at. */
const TAB_LIST_AREAS: readonly SideArea[] = ["left", "right"];

/** What a panel is called in its header. */
function titleOf(panel: Panel, def: PanelDef | undefined): string {
  switch (panel.content.type) {
    case "tab_list":
      return "Tabs";
    case "plugin_view":
      return def?.title ?? panel.content.view;
    case "slot":
      return panel.id;
  }
}

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** A panel in the dock area at `area`. */
export function PanelHost({ panel, area }: { panel: Panel; area: SideArea }) {
  const app = useShell();
  const hideHint = useKeyHint("lume.panel.toggle");
  const held = useRef<{ pointerId: number; x: number; y: number; dragging: boolean } | null>(null);
  const element = useRef<HTMLElement>(null);
  useSlotMotion("panel.enter", element);
  const allowed = panel.id === TAB_LIST_PANEL ? TAB_LIST_AREAS : SIDE_AREAS;
  const def = useStore(app.plugins, (state) =>
    panel.content.type === "plugin_view"
      ? state.shown?.panels.find((one) => one.id === panel.id)
      : undefined,
  );
  const ready = useStore(app.plugins, (state) =>
    def === undefined
      ? false
      : (state.shown?.plugins.find((plugin) => plugin.id === def.plugin)?.ready ?? false),
  );
  const images = useStore(app.plugins, (state) => state.shown?.images);
  const dev = useStore(app.plugins, (state) =>
    def === undefined
      ? false
      : (state.shown?.plugins.find((entry) => entry.id === def.plugin)?.dev ?? false),
  );

  /** The side of the window the pointer holds the panel over. */
  const targetAt = (event: React.PointerEvent<HTMLElement>): SideArea | null => {
    const body = event.currentTarget.closest(".shell-body")?.getBoundingClientRect();
    if (!body) return null;
    return panelDropArea(
      { x: event.clientX, y: event.clientY },
      { x: body.left, y: body.top, width: body.width, height: body.height },
      allowed,
    );
  };

  const moveTo = (target: SideArea) => {
    const state = app.layout.getState();
    if (target === area || state.window === null) return;
    const moved =
      panel.id === TAB_LIST_PANEL
        ? app.client.configSet("layout.tab-placement", target)
        : app.client.layoutApply({
            type: "panel_move",
            window: state.window,
            panel: panel.id,
            area: target,
            index: selectPanels(state, target).length,
          });
    moved.catch((error: unknown) => app.notices.push("error", reason(error)));
  };

  const hide = () => {
    const window = app.layout.getState().window;
    if (window === null) return;
    app.client
      .layoutApply({ type: "panel_toggle", window, area, on: false })
      .catch((error: unknown) => app.notices.push("error", reason(error)));
  };

  const letGo = (event: React.PointerEvent<HTMLElement>, dropped: boolean) => {
    const drag = held.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    held.current = null;
    delete event.currentTarget.dataset["dragging"];
    app.panelDrag.getState().drop();
    if (!dropped || !drag.dragging) return;
    const target = targetAt(event);
    if (target !== null) moveTo(target);
  };

  const closePanel = () => {
    app.commands.run("lume.panel.close", { panel: panel.id });
  };

  return (
    <section ref={element} className="panel" data-panel={panel.id} aria-label={titleOf(panel, def)}>
      <header
        className="panel-header"
        onPointerDown={(event) => {
          if (event.button !== 0 || (event.target as Element).closest("button")) return;
          // The keyboard stays with the terminal while a panel is moved.
          event.preventDefault();
          held.current = {
            pointerId: event.pointerId,
            x: event.clientX,
            y: event.clientY,
            dragging: false,
          };
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          const drag = held.current;
          if (!drag || drag.pointerId !== event.pointerId) return;
          if (!drag.dragging) {
            const far = Math.hypot(event.clientX - drag.x, event.clientY - drag.y);
            if (far < DRAG_START_PX) return;
            drag.dragging = true;
            event.currentTarget.dataset["dragging"] = "true";
          }
          const target = targetAt(event);
          // Where the panel already is is no place to move it to.
          app.panelDrag.getState().hold(panel.id, target === area ? null : target);
        }}
        onPointerUp={(event) => letGo(event, true)}
        onPointerCancel={(event) => letGo(event, false)}
      >
        {def?.icon !== undefined && (
          <span className="panel-icon">
            <PluginIcon icon={def.icon} images={images} size={13} />
          </span>
        )}
        <span className="panel-title">{titleOf(panel, def)}</span>
        {dev && <DevBadge />}
        {panel.content.type === "plugin_view" && (
          <button
            type="button"
            className="control panel-button"
            aria-label="Close this panel"
            title="Close this panel"
            tabIndex={-1}
            onPointerDown={(event) => event.preventDefault()}
            onClick={closePanel}
          >
            <CloseIcon size={11} />
          </button>
        )}
        <button
          type="button"
          className="control panel-button"
          aria-label="Hide this side's panels"
          title={`Hide this side's panels${hideHint}`}
          tabIndex={-1}
          onPointerDown={(event) => event.preventDefault()}
          onClick={hide}
        >
          <CollapseIcon />
        </button>
      </header>
      <div className="panel-body">
        {panel.content.type === "tab_list" && <TabListPanel />}
        {panel.content.type === "plugin_view" &&
          (def !== undefined && ready && def.kind === "list" ? (
            <Suspense fallback={null}>
              <ListPanel panel={def} />
            </Suspense>
          ) : def !== undefined && ready && def.kind === "frame" && def.entry !== undefined ? (
            <PluginFrame
              plugin={def.plugin}
              view={def.view}
              place="panel"
              entry={def.entry}
              title={def.title}
              state={panel.content.state}
              host={panel.id}
            />
          ) : (
            <PluginViewPlaceholder plugin={panel.content.plugin} view={panel.content.view} />
          ))}
      </div>
    </section>
  );
}
