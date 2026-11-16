/**
 * The application's menu, at the left end of the top bar.
 *
 * A button with three lines opens it. While it is open, the names of its other menus
 * unfold beside the button, and moving the pointer or the arrow keys from one name to
 * the next shows that menu in place of the one on show.
 *
 * Every item runs a command, so the menu is a way to find what the application can
 * do and which key does it: each item shows the key bound to its command. An item
 * whose command needs something asked for, such as a workspace's name, hands it to
 * the command palette, which asks. An item is left out when the application has no
 * such command.
 */

import { useRef, useState } from "react";
import { useStore } from "zustand";

import { keyLabel } from "../commands/store.ts";
import { useShell } from "./context.ts";
import { MenuIcon } from "./icons.tsx";
import { Menu, type MenuItem } from "./Menu.tsx";

/** One of the application's menus. */
type Which = "lume" | "view" | "help";

/** The menus in the order they stand in, with what each is called. */
const MENUS: readonly (readonly [Which, string])[] = [
  ["lume", "Lume"],
  ["view", "View"],
  ["help", "Help"],
];

/** How an item's command is run: at once, or once the palette has asked what it needs. */
type How = "run" | "ask";

/** One row of a menu: the command, what the menu calls it, and how it is run. */
type Row = readonly [command: string, label: string, how?: How];

/**
 * What each menu offers, in groups a line separates. Of the rows of one place in a
 * group given as an array, the first whose command there is stands for the place.
 */
const CONTENTS: Readonly<Record<Which, readonly (readonly (Row | readonly Row[])[])[]>> = {
  lume: [
    [
      ["lume.window.new", "New window"],
      ["lume.tab.new", "New tab"],
    ],
    [
      ["lume.workspace.new", "New workspace…", "ask"],
      ["lume.workspace.switch", "Open workspace…", "ask"],
    ],
    [
      [
        ["lume.settings.open", "Settings"],
        ["lume.config.open", "Open configuration file"],
      ],
    ],
    [["lume.app.quit", "Quit Lume"]],
  ],
  view: [
    [
      ["lume.pane.split-right", "Split right"],
      ["lume.pane.split-down", "Split down"],
    ],
    [
      ["lume.view.zoom-in", "Zoom in"],
      ["lume.view.zoom-out", "Zoom out"],
      ["lume.view.zoom-reset", "Reset zoom"],
    ],
  ],
  help: [
    [["lume.app.whats-new", "What's new"]],
    [
      ["lume.app.report-issue", "Report an issue"],
      ["lume.app.show-logs", "Show logs"],
    ],
  ],
};

/**
 * A key as a menu writes it: the keys of a chord set apart by spaces, as in
 * `Ctrl Shift N`. Where chords are written without anything between their keys, as on
 * a Mac, there is nothing to change.
 */
function menuKey(label: string | undefined): string | undefined {
  // A `+` after a key's name joins; one after another `+` is the key itself.
  return label?.replace(/(?<=[^+\s])\+(?=.)/g, " ");
}

function isRow(place: Row | readonly Row[]): place is Row {
  return typeof place[0] === "string";
}

/** The button that opens the application's menu, and the menu. */
export function AppMenu() {
  const app = useShell();
  const known = useStore(app.commandStore, (state) => state.byId);
  const config = useStore(app.config, (state) => state.view);
  const [open, setOpen] = useState<Which | null>(null);
  const button = useRef<HTMLButtonElement>(null);

  const close = () => {
    setOpen(null);
    // The keyboard goes back to the terminal, or to the button when there is none.
    if (app.views.focused()) app.views.focus();
    else button.current?.focus();
  };

  const itemsOf = (which: Which): MenuItem[] => {
    const items: MenuItem[] = [];
    for (const group of CONTENTS[which]) {
      let first = true;
      for (const place of group) {
        const row = isRow(place) ? place : place.find(([command]) => known.has(command));
        if (!row || !known.has(row[0])) continue;
        const [command, label, how = "run"] = row;
        items.push({
          key: command,
          label,
          hint: config
            ? menuKey(keyLabel(config.keymap.bindings, command, config.platform))
            : undefined,
          separated: first && items.length > 0,
          choose: () => {
            if (how === "ask") app.palette.getState().openFor(command);
            else app.commands.run(command);
          },
        });
        first = false;
      }
    }
    return items;
  };

  /** Shows the menu beside the one on show. */
  const step = (by: number) => {
    const at = MENUS.findIndex(([which]) => which === open);
    const next = MENUS[(at + by + MENUS.length) % MENUS.length];
    if (next) setOpen(next[0]);
  };

  const shown = MENUS.find(([which]) => which === open);
  return (
    <div className="app-menu">
      <button
        ref={button}
        type="button"
        className="control app-menu-button"
        aria-haspopup="menu"
        aria-expanded={open === "lume"}
        aria-label="Lume menu"
        title="Menu"
        data-open={open !== null}
        onClick={() => (open === null ? setOpen("lume") : close())}
        onPointerEnter={() => {
          if (open !== null) setOpen("lume");
        }}
      >
        <MenuIcon />
      </button>
      <div className="app-menu-fold" data-open={open !== null} aria-hidden={open === null}>
        <div className="app-menu-bar" role="menubar" aria-label="Application menu">
          {MENUS.slice(1).map(([which, label]) => (
            <button
              key={which}
              type="button"
              className="app-menu-name"
              role="menuitem"
              aria-haspopup="menu"
              aria-expanded={open === which}
              tabIndex={-1}
              // The keyboard stays in the menu on show until this one is.
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => setOpen(which)}
              onPointerEnter={() => {
                if (open !== null) setOpen(which);
              }}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      {shown && (
        <div className="app-menu-drop" data-menu={shown[0]}>
          <Menu
            key={shown[0]}
            label={shown[1]}
            items={itemsOf(shown[0])}
            close={close}
            side={step}
          />
        </div>
      )}
    </div>
  );
}
