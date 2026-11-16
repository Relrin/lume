/**
 * The panels docked at one side of the window.
 *
 * A side with panels takes a strip of the window, as wide (or, at the bottom, as high)
 * as the layout says; a side that is folded away or has no panels takes none. The
 * strip's inner edge is dragged to resize it. While it is dragged the strip follows
 * the pointer through its style, with nothing rendered; the core is told a few times a
 * second and once more when the pointer lets go.
 */

import { useLayoutEffect, useRef } from "react";
import { useStore } from "zustand";

import type { SideArea } from "../drag/panels.ts";
import { selectDock } from "../layout/selectors.ts";
import {
  DOCK_INTENT_INTERVAL_MS,
  DOCK_SIZE_MAX_PX,
  DOCK_SIZE_MIN_PX,
  DRAG_SETTLE_TIMEOUT_MS,
} from "../tuning.ts";
import { useShell } from "./context.ts";
import { PanelHost } from "./PanelHost.tsx";

/** What a side of the window is called. */
const NAMES: Readonly<Record<SideArea, string>> = {
  left: "Left panels",
  right: "Right panels",
  bottom: "Bottom panels",
};

/** A strip's edge that the pointer has hold of. */
interface Resize {
  readonly pointerId: number;
  /** Where the pointer was across the edge when it took hold. */
  readonly start: number;
  /** How far the strip reached then. */
  readonly size: number;
  /** How far it reaches now. */
  now: number;
  /** When the core was last told, on the page's clock, and what. */
  told: number;
  sent: number | undefined;
}

function clamp(size: number): number {
  return Math.round(Math.min(Math.max(size, DOCK_SIZE_MIN_PX), DOCK_SIZE_MAX_PX));
}

/** The dock area at one side of the window, if it has anything on show. */
export function DockArea({ area }: { area: SideArea }) {
  const app = useShell();
  const dock = useStore(app.layout, (state) => (state.ready ? selectDock(state, area) : undefined));
  const strip = useRef<HTMLElement>(null);
  const resize = useRef<Resize | null>(null);
  /** The size the pointer left the strip at, until the layout says the same. */
  const settling = useRef<{ size: number; timer: number } | null>(null);

  const shown = dock !== undefined && !dock.collapsed && dock.panels.length > 0;
  const size = dock?.size ?? 0;
  const across = area === "bottom";

  const write = (extent: number) => {
    const element = strip.current;
    if (!element) return;
    // Through the style object, which a policy that forbids style attributes allows.
    if (across) element.style.height = `${extent}px`;
    else element.style.width = `${extent}px`;
  };

  useLayoutEffect(() => {
    if (!shown || resize.current) return;
    const settle = settling.current;
    if (settle) {
      // An answer to an earlier position of the drag is passed over.
      if (settle.size !== size) return;
      window.clearTimeout(settle.timer);
      settling.current = null;
    }
    write(size);
  });

  if (!shown) return null;

  const tell = (drag: Resize, now: number): void => {
    drag.told = now;
    const window = app.layout.getState().window;
    if (drag.sent === drag.now || window === null) return;
    drag.sent = drag.now;
    app.client
      .layoutApply({ type: "panel_resize", window, area, size: drag.now })
      // A refusal while dragging is passed over: the next size is told anyway.
      .catch(() => undefined);
  };

  const letGo = (event: React.PointerEvent<HTMLElement>) => {
    const drag = resize.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    resize.current = null;
    delete event.currentTarget.dataset["dragging"];
    if (drag.now === drag.size) return;
    tell(drag, performance.now());
    // The strip stays as the pointer left it until the layout says the same, or for
    // as long as an answer may reasonably take.
    const timer = window.setTimeout(() => {
      settling.current = null;
      write(app.layout.getState().ready ? (selectDock(app.layout.getState(), area)?.size ?? 0) : 0);
    }, DRAG_SETTLE_TIMEOUT_MS);
    settling.current = { size: drag.now, timer };
  };

  return (
    <aside ref={strip} className="dock-area" data-area={area} aria-label={NAMES[area]}>
      {dock.panels.map((panel) => (
        <PanelHost key={panel.id} panel={panel} area={area} />
      ))}
      <div
        className="dock-resize"
        role="separator"
        aria-orientation={across ? "horizontal" : "vertical"}
        aria-label={`Resize ${NAMES[area].toLowerCase()}`}
        title="Drag to resize"
        onPointerDown={(event) => {
          const element = strip.current;
          if (event.button !== 0 || resize.current || !element) return;
          event.preventDefault();
          event.currentTarget.setPointerCapture(event.pointerId);
          event.currentTarget.dataset["dragging"] = "true";
          if (settling.current) window.clearTimeout(settling.current.timer);
          settling.current = null;
          const box = element.getBoundingClientRect();
          const extent = Math.round(across ? box.height : box.width);
          resize.current = {
            pointerId: event.pointerId,
            start: across ? event.clientY : event.clientX,
            size: extent,
            now: extent,
            told: performance.now(),
            sent: undefined,
          };
        }}
        onPointerMove={(event) => {
          const drag = resize.current;
          if (!drag || drag.pointerId !== event.pointerId) return;
          const moved = (across ? event.clientY : event.clientX) - drag.start;
          // The left strip grows as its edge goes right; the others as theirs go the
          // other way.
          drag.now = clamp(drag.size + (area === "left" ? moved : -moved));
          write(drag.now);
          const now = performance.now();
          if (now - drag.told >= DOCK_INTENT_INTERVAL_MS) tell(drag, now);
        }}
        onPointerUp={letGo}
        onPointerCancel={letGo}
      />
    </aside>
  );
}

/** Shows the side of the window a panel that is being dragged would be docked at. */
export function DockDropPreview() {
  const app = useShell();
  const target = useStore(app.panelDrag, (state) => state.target);
  if (target === null) return null;
  return <div className="dock-drop" data-area={target} aria-hidden="true" />;
}
