/**
 * The window's own buttons, for a window whose title bar the page draws: put the
 * window away, let it fill the screen or put it back, close it.
 *
 * The buttons only ask. What the window is doing is the core's to say, and the middle
 * button shows whichever of its two faces fits what the core last said.
 */

import { useStore } from "zustand";

import type { WindowAction } from "../core/types.ts";
import { useShell } from "./context.ts";
import { CloseIcon, MaximiseIcon, MinimiseIcon, RestoreIcon } from "./icons.tsx";

/** The three buttons at the right end of the top bar. */
export function WindowControls() {
  const app = useShell();
  const maximised = useStore(app.windowState, (state) => state.maximised);
  const ask = (action: WindowAction) => () => {
    app.client.windowControl(action).catch((error: unknown) => {
      app.notices.push("error", error instanceof Error ? error.message : String(error));
    });
  };
  return (
    <div className="window-controls">
      <button
        type="button"
        className="control window-control"
        aria-label="Minimise"
        title="Minimise"
        tabIndex={-1}
        onPointerDown={(event) => event.preventDefault()}
        onClick={ask("minimise")}
      >
        <MinimiseIcon />
      </button>
      <button
        type="button"
        className="control window-control"
        aria-label={maximised ? "Restore" : "Maximise"}
        title={maximised ? "Restore" : "Maximise"}
        tabIndex={-1}
        onPointerDown={(event) => event.preventDefault()}
        onClick={ask("maximise")}
      >
        {maximised ? <RestoreIcon /> : <MaximiseIcon />}
      </button>
      <button
        type="button"
        className="control window-control"
        aria-label="Close window"
        title="Close window"
        tabIndex={-1}
        onPointerDown={(event) => event.preventDefault()}
        onClick={ask("close")}
      >
        <CloseIcon size={12} />
      </button>
    </div>
  );
}
