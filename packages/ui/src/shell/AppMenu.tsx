/**
 * The application's menu, at the left end of the top bar.
 *
 * A button with three lines opens it. While it is open, the names of its other menus
 * unfold beside the button, and moving the pointer or the arrow keys from one name to
 * the next shows that menu in place of the one on show. Plugins may add menus of their
 * own among them, and items and sections to each, where their manifests ask.
 *
 * Every item runs a command, so the menu is a way to find what the application can
 * do and which key does it: each item shows the key bound to its command. An item
 * whose command needs something asked for, such as a workspace's name, hands it to
 * the command palette, which asks. An item is left out when the application has no
 * such command.
 *
 * Below "New tab" stands "New tab with", whose own menu opens a tab with a profile
 * other than the default: the default first, then the others, then the way to the
 * palette that finds any of them. Below that, while there are groups of tabs that were
 * kept when they were closed, "Groups" lists them to open again. The View menu lists
 * the plugins' panels there are to show.
 */

import { useRef, useState } from "react";
import { useStore } from "zustand";

import { keyLabel } from "../commands/store.ts";
import { KindIcon } from "../connect/icons.tsx";
import { kindOf } from "../connect/model.ts";
import { NEW_TAB_MENU_MAX } from "../connect/tuning.ts";
import type { MenuNodeData, ProfileList } from "../core/types.ts";
import { selectShelf } from "../layout/selectors.ts";
import type { Shelved } from "../layout/types.ts";
import { profileGroup } from "../palette/choices.ts";
import { PluginIcon } from "../plugins/PluginIcon.tsx";
import { useShell } from "./context.ts";
import { menuKey } from "./hints.ts";
import { GroupGlyph, MenuIcon } from "./icons.tsx";
import { Menu, type MenuItem } from "./Menu.tsx";
import { menuRows, nodesOf, type MenuName, type MenuParts } from "./menus.ts";
import { tabColorValue, tabCount, UNNAMED_GROUP } from "./tab-looks.ts";

/** What each of the application's own menus is called, and the menu its items are in. */
const OWN_MENUS: Readonly<Record<string, readonly [label: string, menu: MenuName]>> = {
  lume: ["Lume", "menu.app.lume"],
  view: ["View", "menu.app.view"],
  help: ["Help", "menu.app.help"],
};

/** How an item's command is run: at once, or once the palette has asked what it needs. */
type How = "run" | "ask";

/**
 * The commands of the application's own items, by the item's key: what the menu calls
 * each, and how it is run. Of several commands given for one item, the first there is
 * stands for it.
 */
const OWN_ITEMS: Readonly<Record<string, readonly (readonly [string, string, How?])[]>> = {
  "new-window": [["lume.window.new", "New window"]],
  "new-tab": [["lume.tab.new", "New tab"]],
  "new-workspace": [["lume.workspace.new", "New workspace…", "ask"]],
  "open-folder": [["lume.workspace.open-folder", "Open folder…"]],
  "open-workspace": [["lume.workspace.switch", "Open workspace…", "ask"]],
  settings: [
    ["lume.settings.open", "Settings"],
    ["lume.config.open", "Open configuration file"],
  ],
  detached: [["lume.session.detached", "Detached sessions…"]],
  quit: [["lume.app.quit", "Quit Lume"]],
  "quit-end-sessions": [["lume.app.quit-end-sessions", "Quit and end sessions"]],
  "split-right": [["lume.pane.split-right", "Split right"]],
  "split-down": [["lume.pane.split-down", "Split down"]],
  "zoom-in": [["lume.view.zoom-in", "Zoom in"]],
  "zoom-out": [["lume.view.zoom-out", "Zoom out"]],
  "zoom-reset": [["lume.view.zoom-reset", "Reset zoom"]],
  "about-lume": [["lume.app.about", "About Lume"]],
  "whats-new": [["lume.app.whats-new", "What's new"]],
  commands: [["lume.app.commands", "Commands and keys"]],
  "report-issue": [["lume.app.report-issue", "Report an issue"]],
  "show-logs": [["lume.app.show-logs", "Show logs"]],
  "restart-host": [["lume.host.restart", "Restart session host"]],
};

/** The commands that end programs, which the menu marks as such. */
const ENDS_PROGRAMS: ReadonlySet<string> = new Set([
  "lume.app.quit-end-sessions",
  "lume.host.restart",
]);

/** The size of the pictures of the kinds of terminal in the menu of "New tab with". */
const KIND_ICON_PX = 14;

/** The row of "New tab with" that opens the default terminal, with what it is. */
function defaultTerminalItem(list: ProfileList | null, open: (profile: string | null) => void) {
  const visible = (list?.profiles ?? []).filter((profile) => !profile.hidden);
  const fallback = visible.find((profile) => profile.id === list?.default);
  const item: MenuItem = {
    key: "default",
    label: "Default terminal",
    hint: fallback?.name ?? "System default",
    icon: <KindIcon kind={fallback ? kindOf(fallback) : "local"} size={KIND_ICON_PX} />,
    choose: () => open(null),
  };
  return item;
}

/** The rows of "New tab with" for the other profiles there are to list, each with its kind. */
function profileItems(
  list: ProfileList | null,
  open: (profile: string | null) => void,
): MenuItem[] {
  const visible = (list?.profiles ?? []).filter((profile) => !profile.hidden);
  const fallback = visible.find((profile) => profile.id === list?.default);
  return visible
    .filter((candidate) => candidate !== fallback)
    .slice(0, NEW_TAB_MENU_MAX)
    .map((profile) => ({
      key: `profile:${profile.id}`,
      label: profile.name,
      hint: profileGroup(profile),
      icon: <KindIcon kind={kindOf(profile)} size={KIND_ICON_PX} />,
      choose: () => open(profile.id),
    }));
}

/** The row of "New tab with" that goes to all of them, with the keys that do too. */
function moreItem(more: () => void, moreHint?: string): MenuItem {
  return {
    key: "more",
    label: "More…",
    ...(moreHint === undefined || moreHint === "" ? {} : { hint: moreHint }),
    choose: more,
  };
}

/**
 * The menu of "New tab with": the default terminal, with what it is; the other
 * profiles there are to list, each with its kind; and the way to all of them, with the
 * keys that take it there too. `open` is given a profile's id, or `null` for the
 * default.
 */
export function newTabWithItems(
  list: ProfileList | null,
  open: (profile: string | null) => void,
  more: () => void,
  moreHint?: string,
): MenuItem[] {
  const profiles = profileItems(list, open);
  return [
    defaultTerminalItem(list, open),
    ...profiles.map((item, index) => (index === 0 ? { ...item, separated: true } : item)),
    { ...moreItem(more, moreHint), separated: true },
  ];
}

/** The menu of "Groups": each kept group that was closed, the most recent first. */
export function shelfItems(shelf: readonly Shelved[], reopen: (group: string) => void): MenuItem[] {
  return [...shelf].reverse().map(({ group, tabs }) => ({
    key: `group:${group.id}`,
    label: group.name ?? UNNAMED_GROUP,
    hint: tabCount(tabs.length),
    icon: (
      <span className="menu-group-icon" style={{ color: tabColorValue(group.color) }}>
        {group.icon === undefined ? (
          <span className="menu-dot" style={{ background: tabColorValue(group.color) }} />
        ) : (
          <GroupGlyph icon={group.icon} size={12} />
        )}
      </span>
    ),
    choose: () => reopen(group.id),
  }));
}

/** One of the menus of the bar: what it is called and its rows. */
interface BarMenu {
  readonly key: string;
  readonly label: string;
  readonly items: () => MenuItem[];
}

/** The button that opens the application's menu, and the menu. */
export function AppMenu() {
  const app = useShell();
  const known = useStore(app.commandStore, (state) => state.byId);
  const config = useStore(app.config, (state) => state.view);
  const profiles = useStore(app.profiles, (state) => state.list);
  const shelf = useStore(app.layout, selectShelf);
  const shown = useStore(app.plugins, (state) => state.shown);
  const contexts = useStore(app.plugins, (state) => state.contexts);
  const [open, setOpen] = useState<string | null>(null);
  const button = useRef<HTMLButtonElement>(null);

  const close = () => {
    setOpen(null);
    // The keyboard goes back to the terminal, or to the button when there is none.
    if (app.views.focused()) app.views.focus();
    else button.current?.focus();
  };

  const hintOf = (command: string) =>
    config ? menuKey(keyLabel(config.keymap.bindings, command, config.platform)) : undefined;

  const parts: MenuParts = {
    contexts,
    builtin: (key) => {
      if (key === "new-tab-with") {
        const items = menuRows(nodesOf(shown?.menus, "menu.new-tab"), newTabParts);
        return [{ key: "new-tab-with", label: "New tab with", items, choose: () => undefined }];
      }
      if (key === "groups") {
        if (shelf.length === 0) return [];
        return [
          {
            key: "groups",
            label: "Groups",
            items: shelfItems(shelf, (group) => app.commands.run("lume.group.reopen", { group })),
            choose: () => undefined,
          },
        ];
      }
      if (key === "panel-list") {
        if (!known.has("lume.panel.show")) return [];
        return (shown?.panels ?? []).map((panel) => ({
          key: `panel:${panel.id}`,
          label: `Show ${panel.title}`,
          icon: <PluginIcon icon={panel.icon} images={shown?.images} />,
          choose: () => app.commands.run("lume.panel.show", { panel: panel.id }),
        }));
      }
      const row = (OWN_ITEMS[key] ?? []).find(([command]) => known.has(command));
      if (!row) return [];
      const [command, label, how = "run"] = row;
      return [
        {
          key: command,
          label,
          hint: hintOf(command),
          ...(ENDS_PROGRAMS.has(command) ? { danger: true } : {}),
          choose: () => {
            if (how === "ask") app.palette.getState().openFor(command);
            else app.commands.run(command);
          },
        },
      ];
    },
    plugin: (node) => pluginItem(node),
  };

  const openProfile = (profile: string | null) =>
    app.commands.run("lume.tab.new", profile === null ? null : { profile });
  const newTabParts: MenuParts = {
    contexts,
    builtin: (key) => {
      switch (key) {
        case "default-terminal":
          return [defaultTerminalItem(profiles, openProfile)];
        case "profile-list":
          return profileItems(profiles, openProfile);
        case "more-profiles":
          return [
            moreItem(
              () => app.commands.run("lume.connect.open"),
              known.has("lume.connect.open") ? hintOf("lume.connect.open") : undefined,
            ),
          ];
        default:
          return [];
      }
    },
    plugin: (node) => pluginItem(node),
  };

  /** A row for a plugin's item: what its manifest calls it, its picture and command. */
  function pluginItem(node: MenuNodeData): MenuItem | undefined {
    const command = node.command;
    if (command === undefined || !known.has(command)) return undefined;
    return {
      key: node.key,
      label: node.title ?? known.get(command)?.title ?? command,
      hint: hintOf(command),
      ...(node.icon ? { icon: <PluginIcon icon={node.icon} images={shown?.images} /> } : {}),
      choose: () => app.commands.run(command, node.args ?? null),
    };
  }

  // The menus of the bar, in their order: the application's and the plugins'.
  const bar: BarMenu[] = [];
  for (const node of nodesOf(shown?.menus, "menu.app")) {
    if (node.plugin === undefined) {
      const own = OWN_MENUS[node.key.slice("lume:".length)];
      if (!own) continue;
      const [label, menu] = own;
      bar.push({
        key: node.key.slice("lume:".length),
        label,
        items: () => menuRows(nodesOf(shown?.menus, menu), parts),
      });
    } else if (node.kind === "submenu" && (node.children ?? []).length > 0) {
      bar.push({
        key: node.key,
        label: node.title ?? node.key,
        items: () => menuRows(node.children ?? [], parts),
      });
    }
  }
  const first = bar[0]?.key ?? "lume";

  /** Shows the menu beside the one on show. */
  const step = (by: number) => {
    const at = bar.findIndex((menu) => menu.key === open);
    const next = bar[(at + by + bar.length) % bar.length];
    if (next) setOpen(next.key);
  };

  const showing = bar.find((menu) => menu.key === open);
  return (
    <div className="app-menu">
      <button
        ref={button}
        type="button"
        className="control app-menu-button"
        aria-haspopup="menu"
        aria-expanded={open === first}
        aria-label="Lume menu"
        title="Menu"
        data-open={open !== null}
        onClick={() => (open === null ? setOpen(first) : close())}
        onPointerEnter={() => {
          if (open !== null) setOpen(first);
        }}
      >
        <MenuIcon />
      </button>
      <div className="app-menu-fold" data-open={open !== null} aria-hidden={open === null}>
        <div className="app-menu-bar" role="menubar" aria-label="Application menu">
          {bar.slice(1).map((menu) => (
            <button
              key={menu.key}
              type="button"
              className="app-menu-name"
              role="menuitem"
              aria-haspopup="menu"
              aria-expanded={open === menu.key}
              tabIndex={-1}
              // The keyboard stays in the menu on show until this one is.
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => setOpen(menu.key)}
              onPointerEnter={() => {
                if (open !== null) setOpen(menu.key);
              }}
            >
              {menu.label}
            </button>
          ))}
        </div>
      </div>
      {showing && (
        <div className="app-menu-drop" data-menu={showing.key}>
          <Menu
            key={showing.key}
            label={showing.label}
            items={showing.items()}
            close={close}
            side={step}
          />
        </div>
      )}
    </div>
  );
}
