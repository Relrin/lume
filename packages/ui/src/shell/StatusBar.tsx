/**
 * The status bar along the bottom of the window.
 *
 * It holds status items at its two ends, and is shown only while it has one, so a
 * window whose items are all in the top bar, or that has none, gives its room to the
 * panes. What opens from an item in it, a menu or a card, opens above the item.
 *
 * A press on the bar that is not on a control leaves the keyboard with the terminal,
 * and Escape gives it back from an item that has it.
 */

import { useStore } from "zustand";

import { StatusList } from "../status/StatusItem.tsx";
import { useShell } from "./context.ts";

/** The parts of the bar a press is meant for. */
const CONTROLS = "button, [role=menu]";

/** The status bar, when it has something to show. */
export function StatusBar() {
  const app = useShell();
  const left = useStore(app.status, (state) => state.bySlot["statusbar.left"]);
  const right = useStore(app.status, (state) => state.bySlot["statusbar.right"]);
  if (left.length === 0 && right.length === 0) return null;
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
      <StatusList items={left} slot="statusbar.left" />
      <span className="status-bar-gap" />
      <StatusList items={right} slot="statusbar.right" />
    </footer>
  );
}
