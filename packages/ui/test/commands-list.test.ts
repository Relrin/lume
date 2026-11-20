import { describe, expect, it } from "vitest";

import { COMMAND_GROUPS, commandGroup, keysOf, listCommands } from "../src/commands/groups.ts";
import type { AboutInfo, CommandSpec, KeyBinding } from "../src/core/types.ts";
import { isTabMuted } from "../src/layout/selectors.ts";
import { createLayoutStore } from "../src/layout/store.ts";
import { debugInfo, debugLines } from "../src/settings/debug-info.ts";
import type { SettingsPage } from "../src/settings/model.ts";
import { ABOUT_PAGE, COMMANDS_PAGE, withOwnPages } from "../src/settings/own-pages.ts";
import { oneTab, pane, viewOf } from "./layout-support.ts";

function spec(id: string, title: string, category = "Tab"): CommandSpec {
  return { id, title, category, runs: "core", palette: true };
}

function binding(keys: string, command: string, args: unknown = null): KeyBinding {
  return { keys: keys.split(" "), command, args, when: [], source: null };
}

describe("the group a command is listed under", () => {
  it("is what the command acts on, by the area of its id", () => {
    expect(commandGroup("lume.session.detached")).toBe("Session");
    expect(commandGroup("lume.connect.open")).toBe("Session");
    expect(commandGroup("lume.workspace.switch")).toBe("Session");
    expect(commandGroup("lume.tab.new")).toBe("Tab");
    expect(commandGroup("lume.group.mute")).toBe("Tab");
    expect(commandGroup("lume.pane.split-right")).toBe("Pane");
    expect(commandGroup("lume.terminal.copy")).toBe("Pane");
    expect(commandGroup("lume.search.open")).toBe("Pane");
    expect(commandGroup("lume.host.restart")).toBe("Host");
    expect(commandGroup("lume.profile.refresh")).toBe("Host");
    expect(commandGroup("lume.view.zoom-in")).toBe("View");
    expect(commandGroup("lume.panel.toggle")).toBe("View");
    expect(commandGroup("lume.window.new")).toBe("View");
    expect(commandGroup("lume.palette.open")).toBe("View");
    expect(commandGroup("lume.app.quit")).toBe("App");
    expect(commandGroup("lume.config.reload")).toBe("App");
    expect(commandGroup("lume.settings.open")).toBe("App");
  });

  it("is the tab for the command that opens one, and the application for the rest", () => {
    expect(commandGroup("lume.terminal.new-with-profile")).toBe("Tab");
    expect(commandGroup("lume.somethingnew.do")).toBe("App");
    expect(commandGroup("acme.tools.copy-name")).toBe("App");
  });
});

describe("the keys of a command", () => {
  const bindings = [
    binding("ctrl+shift+t", "lume.tab.new"),
    binding("ctrl+alt+1", "lume.tab.activate", { index: 0 }),
    binding("ctrl+alt+2", "lume.tab.activate", { index: 1 }),
    binding("ctrl+shift+c", "lume.terminal.copy"),
    binding("ctrl+insert", "lume.terminal.copy"),
    binding("ctrl+k ctrl+s", "lume.app.commands"),
    binding("ctrl+shift+c", "lume.terminal.copy"),
  ];

  it("are every way to press it, as the platform writes them, each once", () => {
    expect(keysOf(bindings, "lume.tab.new", "windows")).toEqual(["Ctrl+Shift+T"]);
    expect(keysOf(bindings, "lume.tab.new", "macos")).toEqual(["⌃⇧T"]);
    expect(keysOf(bindings, "lume.terminal.copy", "windows")).toEqual([
      "Ctrl+Shift+C",
      "Ctrl+Insert",
    ]);
    expect(keysOf(bindings, "lume.tab.activate", "linux")).toEqual(["Ctrl+Alt+1", "Ctrl+Alt+2"]);
    expect(keysOf(bindings, "lume.app.commands", "windows")).toEqual(["Ctrl+K Ctrl+S"]);
    expect(keysOf(bindings, "lume.pane.close", "windows")).toEqual([]);
  });

  it("are listed with every command, in the groups' order and by name within one", () => {
    const commands = [
      spec("lume.app.quit", "Quit", "Application"),
      spec("lume.tab.new", "New Tab"),
      spec("lume.tab.close", "Close Tab"),
      { ...spec("lume.tab.activate", "Go to Tab"), palette: false },
      spec("lume.host.restart", "Restart Session Host", "Host"),
      spec("acme.tools.copy-name", "Copy Tab Name", "Acme Tools"),
    ];
    const listed = listCommands(commands, bindings, "windows");
    expect(listed.map((group) => group.group)).toEqual(["Tab", "Host", "App"]);
    expect(listed.every((group) => COMMAND_GROUPS.includes(group.group))).toBe(true);
    // One that the palette leaves out is listed all the same.
    expect(listed[0]?.commands.map((command) => command.title)).toEqual([
      "Close Tab",
      "Go to Tab",
      "New Tab",
    ]);
    expect(listed[0]?.commands[2]?.keys).toEqual(["Ctrl+Shift+T"]);
    expect(listed[2]?.commands.map((command) => [command.title, command.plugin])).toEqual([
      ["Copy Tab Name", true],
      ["Quit", false],
    ]);
  });

  it("are found by what a command is called, its id, its group or its keys", () => {
    const commands = [
      spec("lume.tab.new", "New Tab"),
      spec("lume.terminal.copy", "Copy", "Terminal"),
      spec("lume.host.restart", "Restart Session Host", "Host"),
    ];
    const titles = (query: string) =>
      listCommands(commands, bindings, "windows", query).flatMap((group) =>
        group.commands.map((command) => command.title),
      );
    expect(titles("new tab")).toEqual(["New Tab"]);
    expect(titles("terminal.copy")).toEqual(["Copy"]);
    expect(titles("ctrl+insert")).toEqual(["Copy"]);
    expect(titles("host")).toEqual(["Restart Session Host"]);
    expect(titles("nothing like it")).toEqual([]);
  });
});

describe("what is copied for a report of a fault", () => {
  const info: AboutInfo = {
    appVersion: "0.3.1",
    os: "windows",
    arch: "x86_64",
    engine: "WebView2",
    engineVersion: "140.0.3485.54",
    proto: 2,
    host: { version: "0.3.0", proto: 2, daemon: true, connected: true },
    rexVersion: "0.4.1",
  };

  it("names the application, the Rex server and the parts by their versions", () => {
    expect(debugInfo(info)).toBe(
      [
        "Lume: 0.3.1",
        "Rex server: 0.4.1",
        "Session host: 0.3.0 (on its own, connected)",
        "Host protocol: 2 (application speaks 2)",
        "Webview: WebView2 140.0.3485.54",
        "System: windows x86_64",
      ].join("\n"),
    );
  });

  it("says so when there is no Rex server, no host, or a webview that does not say", () => {
    const bare = debugInfo({ ...info, rexVersion: null, host: null, engineVersion: null });
    expect(bare).toContain("Rex server: not installed");
    expect(bare).toContain("Session host: none");
    expect(bare).toContain("Webview: WebView2 of an unknown version");
    const inside = debugInfo({
      ...info,
      host: { version: "0.3.1", proto: 2, daemon: false, connected: false },
    });
    expect(inside).toContain("Session host: 0.3.1 (inside the application, not connected)");
  });

  it("is made of nothing but what it was given, a line for each thing", () => {
    // Every value written is one of the report's own: nothing is read from elsewhere,
    // so nothing of the machine, its user or its hosts can be in it.
    const given = new Set(
      [
        ...Object.values(info).filter((value) => typeof value !== "object"),
        ...Object.values(info.host ?? {}),
      ].map(String),
    );
    const words = debugLines(info)
      .flatMap(([, value]) => value.split(/[\s(),]+/))
      .filter((word) => /\d/.test(word) || /^[A-Z]/.test(word) || word.includes("_"));
    for (const word of words) expect(given.has(word)).toBe(true);
    expect(debugLines(info).map(([name]) => name)).toEqual([
      "Lume",
      "Rex server",
      "Session host",
      "Host protocol",
      "Webview",
      "System",
    ]);
  });
});

describe("the settings window's own pages", () => {
  const page = (id: string, group: string): SettingsPage => ({
    id,
    label: id,
    icon: id,
    group,
    blurb: "",
    groups: [],
  });

  it("put what the application is last of its pages, and the keys first of the workspace's", () => {
    const pages = withOwnPages([
      page("general", "Application"),
      page("terminal", "Application"),
      page("workspace", "Workspace"),
    ]);
    expect(pages.map((held) => held.id)).toEqual([
      "general",
      "terminal",
      ABOUT_PAGE,
      COMMANDS_PAGE,
      "workspace",
    ]);
    expect(pages.filter((held) => held.own !== undefined).map((held) => held.group)).toEqual([
      "Application",
      "Workspace",
    ]);
  });

  it("stand in for a page that has one of their ids", () => {
    const pages = withOwnPages([page("about", "Application")]);
    expect(pages.map((held) => held.id)).toEqual([ABOUT_PAGE, COMMANDS_PAGE]);
    expect(pages[0]?.own).toBe("about");
  });
});

describe("a muted tab", () => {
  it("is one that is muted itself, or whose group is", () => {
    const store = createLayoutStore();
    const view = viewOf(oneTab(pane("p1")));
    const tabs = view.workspace.windows[0]!.centre.tabs;
    const first = tabs[0]!;
    store.getState().reset({
      ...view,
      workspace: {
        ...view.workspace,
        windows: [
          {
            ...view.workspace.windows[0]!,
            centre: {
              ...view.workspace.windows[0]!.centre,
              tabs: [{ ...first, group: "g1" }],
              groups: [{ id: "g1", color: "blue" }],
            },
          },
        ],
      },
    });
    const tab = first.id;
    expect(isTabMuted(store.getState(), tab)).toBe(false);

    const { epoch, revision } = store.getState();
    store
      .getState()
      .apply(epoch, revision + 1, [
        { type: "tab_state", tab, color: null, locked: false, muted: true, group: "g1" },
      ]);
    expect(store.getState().tabs[tab]?.muted).toBe(true);
    expect(isTabMuted(store.getState(), tab)).toBe(true);

    // Left out of a change, the tab is not muted; its group may still be.
    store.getState().apply(epoch, revision + 2, [
      { type: "tab_state", tab, color: null, locked: false, group: "g1" },
      {
        type: "groups",
        window: store.getState().tabs[tab]!.window,
        groups: [{ id: "g1", color: "blue", muted: true }],
      },
    ]);
    expect(store.getState().tabs[tab]?.muted).toBe(false);
    expect(isTabMuted(store.getState(), tab)).toBe(true);
    expect(isTabMuted(store.getState(), "no-such-tab")).toBe(false);
  });
});
