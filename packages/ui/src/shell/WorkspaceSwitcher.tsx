/**
 * What the window shows, and the way to something else.
 *
 * A button at the left end of the top bar names the host and the workspace or session of
 * it that is on show, with how near a host elsewhere is. While a connection is being made
 * again the dot before the host's name is a ring that turns. Pressed, the button opens the
 * selector of hosts and what they hold.
 */

import { useCallback, useRef, useState } from "react";
import { useStore } from "zustand";

import { hostOnShow } from "../rex/model.ts";
import { useShell } from "./context.ts";
import { selectorLabel } from "./host-selector.ts";
import { HostSelector } from "./HostSelector.tsx";
import { ChevronDownIcon } from "./icons.tsx";

/** The button that names what is on show, and the selector under it. */
export function WorkspaceSwitcher() {
  const app = useShell();
  const name = useStore(app.layout, (state) => state.name);
  const backend = useStore(app.layout, (state) => state.backend);
  const server = useStore(app.rex, (state) => hostOnShow(state.list, backend));
  const connected = useStore(app.hostLink, (state) => state.connected);
  // How it was opened: from the keyboard it opens without motion.
  const [open, setOpen] = useState<"keyboard" | "pointer" | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const label = selectorLabel(name, backend, server, connected);

  const dismiss = useCallback(() => setOpen(null), []);
  const close = useCallback(() => {
    setOpen(null);
    // The keyboard goes back to the terminal, or to the button when there is none.
    if (app.views.focused()) app.views.focus();
    else button.current?.focus();
  }, [app]);

  return (
    <div className="switcher">
      <button
        ref={button}
        type="button"
        className="switcher-button"
        aria-haspopup="dialog"
        aria-expanded={open !== null}
        aria-label={`${label.host}, ${label.name}`}
        aria-description={label.meta === "" ? label.said : `${label.said}, ${label.meta}`}
        data-served={backend !== null || undefined}
        title={`${label.host} / ${label.name}\n${label.said}`}
        onClick={(event) => {
          if (open !== null) close();
          else setOpen(event.detail === 0 ? "keyboard" : "pointer");
        }}
      >
        {/* The dot is said in the button's description, not in its name. */}
        <span className="switcher-dot" data-link={label.link} aria-hidden="true" />
        <span className="switcher-host">{label.host}</span>
        <span className="switcher-slash" aria-hidden="true">
          /
        </span>
        <span className="switcher-name">{label.name}</span>
        {label.meta !== "" && <span className="switcher-meta">{label.meta}</span>}
        <span className="switcher-chevron">
          <ChevronDownIcon />
        </span>
      </button>
      {open !== null && (
        <HostSelector keyboard={open === "keyboard"} close={close} dismiss={dismiss} />
      )}
    </div>
  );
}
