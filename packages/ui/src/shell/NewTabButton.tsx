/**
 * The button that opens another tab, as the row of tabs and the column of tabs show it,
 * and the menu beside it that opens one from another profile.
 *
 * The button opens a tab with the default profile. The menu lists the default first,
 * ticked, then the other profiles there are to list, and a way to the palette that
 * finds any of them.
 */

import { useRef, useState } from "react";
import { useStore } from "zustand";

import { NEW_TAB_MENU_MAX } from "../connect/tuning.ts";
import type { ProfileList } from "../core/types.ts";
import { profileGroup } from "../palette/choices.ts";
import { useShell } from "./context.ts";
import { useKeyHint } from "./hints.ts";
import { ChevronDownIcon, PlusIcon } from "./icons.tsx";
import { Menu, type MenuItem } from "./Menu.tsx";

/** What the menu offers: the default profile, some others, and the way to all of them. */
export function newTabItems(
  list: ProfileList | null,
  open: (profile: string | null) => void,
  more: () => void,
): MenuItem[] {
  const visible = (list?.profiles ?? []).filter((profile) => !profile.hidden);
  const fallback = visible.find((profile) => profile.id === list?.default);
  const items: MenuItem[] = [
    fallback
      ? {
          key: `profile:${fallback.id}`,
          label: fallback.name,
          hint: profileGroup(fallback),
          checked: true,
          choose: () => open(fallback.id),
        }
      : { key: "default", label: "Default shell", checked: true, choose: () => open(null) },
  ];
  for (const profile of visible.filter((candidate) => candidate !== fallback)) {
    if (items.length > NEW_TAB_MENU_MAX) break;
    items.push({
      key: `profile:${profile.id}`,
      label: profile.name,
      hint: profileGroup(profile),
      checked: false,
      choose: () => open(profile.id),
    });
  }
  items.push({ key: "more", label: "More…", separated: true, choose: more });
  return items;
}

/**
 * Opens a tab with the default profile, or another from its menu. `column` says the
 * tabs are listed down the side of the window, where the button is a row with its name
 * beside the picture.
 */
export function NewTabButton({ column = false }: { column?: boolean }) {
  const app = useShell();
  const newHint = useKeyHint("lume.tab.new");
  const connectHint = useKeyHint("lume.connect.open");
  const list = useStore(app.profiles, (state) => state.list);
  const [open, setOpen] = useState(false);
  const chevron = useRef<HTMLButtonElement>(null);

  const close = () => {
    setOpen(false);
    // The keyboard goes back to the terminal, or to the button when there is none.
    if (app.views.focused()) app.views.focus();
    else chevron.current?.focus();
  };
  const items = open
    ? newTabItems(
        list,
        (profile) => app.commands.run("lume.tab.new", profile === null ? null : { profile }),
        () => app.commands.run("lume.connect.open"),
      )
    : [];
  // The way to every profile says which keys take it there too.
  const more = items.at(-1);
  if (more && connectHint !== "") {
    items[items.length - 1] = { ...more, hint: connectHint.trim().replace(/^\(|\)$/g, "") };
  }

  return (
    <div className={column ? "tab-column-new-group" : "tab-new-group"}>
      <button
        type="button"
        className={column ? "control tab-column-new" : "control tab-new"}
        aria-label="New tab"
        title={`New tab${newHint}`}
        onPointerDown={(event) => event.preventDefault()}
        onClick={() => app.commands.run("lume.tab.new")}
      >
        <PlusIcon />
        {column && <span>New tab</span>}
      </button>
      <button
        ref={chevron}
        type="button"
        className="control tab-new-menu"
        aria-label="New tab with a profile"
        aria-haspopup="menu"
        aria-expanded={open}
        title="New tab with a profile"
        onPointerDown={(event) => event.preventDefault()}
        onClick={() => setOpen(!open)}
      >
        <ChevronDownIcon />
      </button>
      {open && <Menu label="New tab with a profile" items={items} close={close} />}
    </div>
  );
}
