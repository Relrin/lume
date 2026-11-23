/**
 * The status bar along the bottom of the window.
 *
 * It holds status items at its two ends, and while a server's layout is on show it says
 * at its left whose. It is shown only while it has something, so a window whose items
 * are all in the top bar, or that has none, gives its room to the panes. What opens from an item in it, a menu or a card, opens above the item.
 *
 * A press on the bar that is not on a control leaves the keyboard with the terminal,
 * and Escape gives it back from an item that has it.
 */

import { useStore } from "zustand";

import { selectActivePanes, selectFloating } from "../layout/selectors.ts";
import { LinkDot } from "../rex/LinkDot.tsx";
import { hostOnShow } from "../rex/model.ts";
import { StatusList } from "../status/StatusItem.tsx";
import { useShell } from "./context.ts";
import { selectorLabel } from "./host-selector.ts";

/** The parts of the bar a press is meant for. */
const CONTROLS = "button, [role=menu]";

/** The server whose layout is on show, its session, and how many panes of it are. */
function ServedStatus({ backend }: { backend: string }) {
  const app = useShell();
  const name = useStore(app.layout, (state) => state.name);
  const panes = useStore(
    app.layout,
    (state) => selectActivePanes(state).length + selectFloating(state).length,
  );
  const server = useStore(app.rex, (state) => hostOnShow(state.list, backend));
  const label = selectorLabel(name, backend, server, true);
  return (
    <span className="status-served">
      <LinkDot link={label.link} className="switcher-dot" />
      <span className="status-served-text">
        {label.host} · session {name} · {panes === 1 ? "1 pane" : `${panes} panes`}
        {label.link === "down" ? ` · ${label.said.toLowerCase()}` : ""}
      </span>
    </span>
  );
}

/** The status bar, when it has something to show. */
export function StatusBar() {
  const app = useShell();
  const left = useStore(app.status, (state) => state.bySlot["statusbar.left"]);
  const right = useStore(app.status, (state) => state.bySlot["statusbar.right"]);
  const backend = useStore(app.layout, (state) => state.backend);
  if (left.length === 0 && right.length === 0 && backend === null) return null;
  return (
    <footer
      className="status-bar"
      onKeyDown={(event) => {
        if (event.key !== "Escape" || event.defaultPrevented) return;
        if ((event.target as Element).closest("[role=menu]")) return;
        event.preventDefault();
        app.views.focus();
      }}
      onMouseDown={(event) => {
        if (event.button !== 0 || (event.target as Element).closest(CONTROLS)) return;
        event.preventDefault();
      }}
    >
      {backend !== null && <ServedStatus backend={backend} />}
      <StatusList items={left} slot="statusbar.left" />
      <span className="status-bar-gap" />
      <StatusList items={right} slot="statusbar.right" />
    </footer>
  );
}
