import { DEFAULT_TOKENS } from "@lume/tokens";
import { describe, expect, it } from "vitest";

import { chordLabel, createCommandStore, keyLabel } from "../src/commands/store.ts";
import { createConfigStore } from "../src/config/store.ts";
import type { CommandSpec, ConfigView, KeyBinding } from "../src/core/types.ts";
import { settings } from "./config-support.ts";

function view(revision: number): ConfigView {
  return {
    revision,
    platform: "windows",
    scheme: "dark",
    theme: { id: "lume-dark", scheme: "dark" },
    config: settings({
      appearance: { "pane-header": "always" },
      terminal: { "scrollback-lines": 10_000, "cursor-style": "block", "cursor-blink": true },
      layout: { "tab-placement": "top" },
      renderer: { tier: "auto" },
    }),
    sources: {},
    problems: [],
    tokens: DEFAULT_TOKENS,
    keymap: { bindings: [], removed: [] },
    schema_revision: 1,
    title_bar: "system",
  };
}

describe("the configuration a page holds", () => {
  it("holds nothing until it is given a view", () => {
    expect(createConfigStore().getState().view).toBeNull();
  });

  it("takes a newer view and drops one that is not newer", () => {
    const store = createConfigStore();
    const first = view(3);
    expect(store.getState().set(first)).toBe(true);
    expect(store.getState().view).toBe(first);

    // An answer that was overtaken by a notice.
    expect(store.getState().set(view(2))).toBe(false);
    expect(store.getState().set(view(3))).toBe(false);
    expect(store.getState().view).toBe(first);

    const next = view(4);
    expect(store.getState().set(next)).toBe(true);
    expect(store.getState().view).toBe(next);
  });

  it("tells whoever listens only when the view changed", () => {
    const store = createConfigStore();
    let told = 0;
    store.subscribe(() => (told += 1));
    store.getState().set(view(1));
    store.getState().set(view(1));
    store.getState().set(view(2));
    expect(told).toBe(2);
  });
});

const COMMANDS: CommandSpec[] = [
  { id: "lume.tab.new", title: "New Tab", category: "Tab", runs: "core", palette: true },
  { id: "lume.terminal.copy", title: "Copy", category: "Terminal", runs: "page", palette: true },
];

describe("the commands a page knows of", () => {
  it("finds a command by its id", () => {
    const store = createCommandStore();
    expect(store.getState().find("lume.tab.new")).toBeUndefined();
    store.getState().set(COMMANDS);
    expect(store.getState().commands).toBe(COMMANDS);
    expect(store.getState().find("lume.terminal.copy")?.runs).toBe("page");
    expect(store.getState().byId.get("lume.tab.new")?.title).toBe("New Tab");
    expect(store.getState().find("lume.nothing")).toBeUndefined();
  });

  it("replaces the list whole", () => {
    const store = createCommandStore();
    store.getState().set(COMMANDS);
    store.getState().set([COMMANDS[1]!]);
    expect(store.getState().find("lume.tab.new")).toBeUndefined();
    expect(store.getState().commands).toHaveLength(1);
  });
});

function binding(keys: string, command: string, args: unknown = null): KeyBinding {
  return { keys: keys.split(" "), command, args, when: [], source: { layer: "default" } };
}

describe("keys written for people", () => {
  it("writes a chord in words on Windows and Linux", () => {
    expect(chordLabel("ctrl+shift+t", "windows")).toBe("Ctrl+Shift+T");
    expect(chordLabel("ctrl+alt+1", "linux")).toBe("Ctrl+Alt+1");
    expect(chordLabel("shift+pageup", "windows")).toBe("Shift+PageUp");
    expect(chordLabel("ctrl+shift+left", "windows")).toBe("Ctrl+Shift+←");
    expect(chordLabel("ctrl+=", "windows")).toBe("Ctrl+=");
    expect(chordLabel("ctrl++", "windows")).toBe("Ctrl++");
    expect(chordLabel("+", "windows")).toBe("+");
    expect(chordLabel("f5", "linux")).toBe("F5");
    expect(chordLabel("ctrl+shift+,", "linux")).toBe("Ctrl+Shift+,");
    expect(chordLabel("ctrl+shift+enter", "windows")).toBe("Ctrl+Shift+Enter");
  });

  it("writes the logo key by the platform's name for it", () => {
    expect(chordLabel("super+e", "windows")).toBe("Win+E");
    expect(chordLabel("super+e", "linux")).toBe("Super+E");
    expect(chordLabel("super+t", "macos")).toBe("⌘T");
  });

  it("writes a chord in signs on a Mac", () => {
    expect(chordLabel("shift+super+d", "macos")).toBe("⇧⌘D");
    expect(chordLabel("ctrl+alt+shift+super+k", "macos")).toBe("⌃⌥⇧⌘K");
    expect(chordLabel("alt+super+left", "macos")).toBe("⌥⌘←");
    expect(chordLabel("super+,", "macos")).toBe("⌘,");
    expect(chordLabel("ctrl+tab", "macos")).toBe("⌃Tab");
  });

  it("labels a command with its first binding", () => {
    const bindings = [
      binding("ctrl+shift+c", "lume.terminal.copy"),
      binding("ctrl+insert", "lume.terminal.copy"),
      binding("ctrl+a c", "lume.tab.new"),
    ];
    expect(keyLabel(bindings, "lume.terminal.copy", "windows")).toBe("Ctrl+Shift+C");
    expect(keyLabel(bindings, "lume.tab.new", "linux")).toBe("Ctrl+A C");
    expect(keyLabel(bindings, "lume.pane.zoom", "windows")).toBeUndefined();
  });

  it("can ask for the binding that gives a command certain arguments", () => {
    const bindings = [
      binding("ctrl+alt+1", "lume.tab.activate", { index: 1 }),
      binding("ctrl+alt+2", "lume.tab.activate", { index: 2 }),
    ];
    expect(keyLabel(bindings, "lume.tab.activate", "windows", { index: 2 })).toBe("Ctrl+Alt+2");
    expect(keyLabel(bindings, "lume.tab.activate", "windows")).toBe("Ctrl+Alt+1");
    expect(keyLabel(bindings, "lume.tab.activate", "windows", { index: 7 })).toBeUndefined();
  });
});
