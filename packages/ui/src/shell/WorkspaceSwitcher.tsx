/**
 * The workspace on show, and the way to another.
 *
 * A button at the left end of the top bar names the workspace. Pressed, it lists the
 * workspaces there are, for switching to one, and what can be done with them: making
 * one, renaming or deleting one, giving the one on show a folder. Whatever needs a
 * name asked for is handed to the command palette, which asks.
 */

import { useEffect, useRef, useState } from "react";
import { useStore } from "zustand";

import type { WorkspaceList } from "../core/types.ts";
import { useShell } from "./context.ts";
import { ChevronDownIcon } from "./icons.tsx";
import { Menu, type MenuItem } from "./Menu.tsx";

/** The commands the menu hands to the palette, with what the menu calls each. */
const ASKED = [
  ["lume.workspace.new", "New workspace…"],
  ["lume.workspace.rename", "Rename workspace…"],
  ["lume.workspace.set-root", "Set workspace folder…"],
  ["lume.workspace.delete", "Delete workspace…"],
] as const;

/** The last part of a folder's path, which is what tells folders apart at a glance. */
function folderName(path: string): string {
  const parts = path.split(/[\\/]/).filter((part) => part !== "");
  return parts.at(-1) ?? path;
}

/** The button that names the workspace, and its menu. */
export function WorkspaceSwitcher() {
  const app = useShell();
  const name = useStore(app.layout, (state) => state.name);
  const known = useStore(app.commandStore, (state) => state.byId);
  // The dot says whether the session host can be reached, whatever the theme's accent.
  const connected = useStore(app.hostLink, (state) => state.connected);
  const linkLabel = connected ? "Session host connected" : "Session host disconnected";
  const [open, setOpen] = useState(false);
  const [listed, setListed] = useState<WorkspaceList | null>(null);
  const button = useRef<HTMLButtonElement>(null);

  // The list is read each time the menu opens: workspaces are made and removed from
  // other windows and from outside the application.
  useEffect(() => {
    if (!open) return undefined;
    let current = true;
    setListed(null);
    app.client.workspacesList().then(
      (list) => {
        if (current) setListed(list);
      },
      () => {
        // Without the list there is still what can be done with workspaces.
        if (current) setListed({ active: "", workspaces: [] });
      },
    );
    return () => {
      current = false;
    };
  }, [app, open]);

  const close = () => {
    setOpen(false);
    // The keyboard goes back to the terminal, or to the button when there is none.
    if (app.views.focused()) app.views.focus();
    else button.current?.focus();
  };

  const items: MenuItem[] = [];
  if (listed === null) {
    items.push({ key: "loading", label: "Loading…", disabled: true, choose: () => undefined });
  } else {
    for (const workspace of listed.workspaces) {
      const active = workspace.name === listed.active;
      items.push({
        key: `workspace:${workspace.name}`,
        label: workspace.name,
        hint: workspace.root === null ? undefined : folderName(workspace.root),
        checked: active,
        choose: () => {
          if (!active) app.commands.run("lume.workspace.switch", { name: workspace.name });
        },
      });
    }
  }
  let separated = items.length > 0;
  for (const [command, label] of ASKED) {
    if (!known.has(command)) continue;
    items.push({
      key: command,
      label,
      separated,
      choose: () => app.palette.getState().openFor(command),
    });
    separated = false;
  }

  return (
    <div className="switcher">
      <button
        ref={button}
        type="button"
        className="switcher-button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-description={linkLabel}
        title={`Workspace: ${name}\n${linkLabel}`}
        onClick={() => setOpen(!open)}
      >
        {/* The dot is said in the button's description, not in its name. */}
        <span className="switcher-dot" data-link={connected ? "up" : "down"} aria-hidden="true" />
        <span className="switcher-name">{name}</span>
        <span className="switcher-chevron">
          <ChevronDownIcon />
        </span>
      </button>
      {open && <Menu label="Workspaces" items={items} close={close} />}
    </div>
  );
}
