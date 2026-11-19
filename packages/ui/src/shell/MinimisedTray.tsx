/**
 * The panes that are put away, as a row of markers at the bottom right of the window.
 *
 * A pane that is put away keeps its terminal and its program; only its card is gone.
 * Its marker names it and brings it back when pressed: to float where it floated, or
 * among the tabs again if that is where it was taken from.
 */

import { useRef } from "react";
import { useStore } from "zustand";

import { selectMinimised } from "../layout/selectors.ts";
import { useSlotMotion } from "../motion/slots.ts";
import type { PaneId } from "../layout/types.ts";
import { useShell } from "./context.ts";
import { ProgramTile } from "./icons.tsx";
import { usePaneName, useProgramIcon } from "./PaneCard.tsx";

/** No panes: one list for every window that has none put away. */
const NONE: readonly PaneId[] = [];

/** The marker of one pane that is put away. */
function MinimisedPill({ pane }: { pane: PaneId }) {
  const app = useShell();
  const pill = useRef<HTMLButtonElement>(null);
  useSlotMotion("pill.enter", pill);
  const name = usePaneName(app, pane, pill);
  const icon = useProgramIcon(app, pane);
  return (
    <button
      ref={pill}
      type="button"
      className="pane-pill"
      title="Restore pane"
      // The keyboard goes to the pane that comes back, not to its marker.
      onPointerDown={(event) => event.preventDefault()}
      onClick={() => {
        app.client
          .layoutApply({ type: "minimise", pane, on: false })
          .catch((error: unknown) =>
            app.notices.push("error", error instanceof Error ? error.message : String(error)),
          );
      }}
    >
      <span className="pane-pill-icon">
        <ProgramTile tile={icon} place="tray" />
      </span>
      <span className="pane-pill-name" ref={name} />
      <span className="pane-pill-mark" aria-hidden="true" />
    </button>
  );
}

/** The markers of this window's panes that are put away. */
export function MinimisedTray() {
  const app = useShell();
  const minimised = useStore(app.layout, (state) => (state.ready ? selectMinimised(state) : NONE));
  if (minimised.length === 0) return null;
  return (
    <div className="pane-tray" role="group" aria-label="Minimised panes">
      {minimised.map((pane) => (
        <MinimisedPill key={pane} pane={pane} />
      ))}
    </div>
  );
}
