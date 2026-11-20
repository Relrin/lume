import { describe, expect, it } from "vitest";

import type { CommandSpec, KeyBinding } from "../src/core/types.ts";
import {
  addKey,
  keptBy,
  keyCommands,
  keyMatches,
  keyPath,
  replaceKey,
  resetAll,
  resetCommand,
  restoreKey,
  unbindKey,
  undoOf,
  type KeyCommand,
  type KeyContext,
} from "../src/settings/keybindings.ts";

const FILE = "C:/Users/kai/.config/lume/config.toml";

function spec(id: string, title: string): CommandSpec {
  return { id, title, category: "Tab", runs: "core", palette: true };
}

const DEFAULT = { layer: "default" };
const user = (line: number) => ({ layer: "user", file: FILE, line });

function binding(
  keys: string,
  command: string,
  source: unknown = DEFAULT,
  when: readonly string[] = [],
  args: unknown = null,
): KeyBinding {
  return { keys: keys.split(" "), command, args, when, source };
}

const SPECS = [
  spec("lume.tab.new", "New Tab"),
  spec("lume.tab.close", "Close Tab"),
  spec("lume.terminal.copy", "Copy"),
  spec("lume.workspace.rename", "Rename Workspace"),
];

/**
 * A user who added Ctrl+Alt+T for a new tab, unbound the default of closing one, and
 * set a key for renaming the workspace that Copy has in the terminal too.
 */
function context(): KeyContext {
  return {
    platform: "windows",
    keymap: {
      bindings: [
        binding("ctrl+shift+t", "lume.tab.new"),
        binding("ctrl+alt+t", "lume.tab.new", user(3)),
        binding("ctrl+shift+c", "lume.terminal.copy", DEFAULT, ["terminal"]),
        binding("ctrl+shift+c", "lume.workspace.rename", user(5)),
      ],
      removed: [{ binding: binding("ctrl+shift+w", "lume.tab.close"), by: user(4) }],
    },
    sources: {
      'keybindings."ctrl+alt+t"': user(3),
      'keybindings."ctrl+shift+w"': user(4),
      'keybindings."ctrl+shift+c"': user(5),
      "font.size": user(9),
    },
    table: {
      "ctrl+alt+t": "lume.tab.new",
      "ctrl+shift+w": "unbound",
      "ctrl+shift+c": "lume.workspace.rename",
    },
  };
}

function command(id: string, held = context()): KeyCommand {
  const found = keyCommands(SPECS, held).find((candidate) => candidate.id === id);
  if (!found) throw new Error(`no command ${id}`);
  return found;
}

describe("the keys of every command", () => {
  it("say where each comes from, with a default the user unbound struck through", () => {
    const newTab = command("lume.tab.new");
    expect(newTab.keys.map((key) => [key.label, key.source, key.off])).toEqual([
      ["Ctrl+Shift+T", "default", false],
      ["Ctrl+Alt+T", "user", false],
    ]);
    expect(newTab.keys[1]?.path).toBe('keybindings."ctrl+alt+t"');
    expect(newTab.changed).toBe(true);

    const close = command("lume.tab.close");
    expect(close.keys.map((key) => [key.label, key.off, key.path])).toEqual([
      ["Ctrl+Shift+W", true, 'keybindings."ctrl+shift+w"'],
    ]);
    expect(close.changed).toBe(true);
    expect(close.defaults).toEqual(["ctrl+shift+w"]);
  });

  it("mark keys two commands share in the same place", () => {
    expect(command("lume.workspace.rename").shared).toBe(true);
    expect(command("lume.workspace.rename").keys[0]?.shared).toEqual(["lume.terminal.copy"]);
    expect(command("lume.terminal.copy").shared).toBe(true);
    expect(command("lume.tab.new").shared).toBe(false);
  });

  it("are narrowed by what is typed, and by the filters", () => {
    const all = keyCommands(SPECS, context());
    expect(all.filter((found) => keyMatches(found, "alt+t")).map((found) => found.id)).toEqual([
      "lume.tab.new",
    ]);
    expect(all.filter((found) => keyMatches(found, "tab close")).map((found) => found.id)).toEqual([
      "lume.tab.close",
    ]);
    const ids = (filter: Parameters<typeof keptBy>[0]) =>
      all.filter((found) => keptBy(filter, found)).map((found) => found.id);
    expect(ids("off")).toEqual(["lume.tab.close"]);
    expect(ids("none")).toEqual(["lume.tab.close"]);
    expect(ids("shared").sort()).toEqual(["lume.terminal.copy", "lume.workspace.rename"]);
    expect(ids("changed").sort()).toEqual([
      "lume.tab.close",
      "lume.tab.new",
      "lume.workspace.rename",
    ]);
  });
});

describe("changing a command's keys", () => {
  it("writes a new key, or has back a default that was unbound", () => {
    const held = context();
    expect(addKey(command("lume.tab.new"), "ctrl+alt+n", "keep", held)).toEqual([
      { op: "set", path: keyPath("ctrl+alt+n"), value: "lume.tab.new" },
    ]);
    expect(addKey(command("lume.tab.close"), "ctrl+shift+w", "keep", held)).toEqual([
      { op: "unset", path: 'keybindings."ctrl+shift+w"' },
    ]);
  });

  it("keeps a key for both commands, or moves it, where another has it", () => {
    const held = context();
    const close = command("lume.tab.close");
    expect(addKey(close, "ctrl+shift+t", "keep", held)).toEqual([
      {
        op: "set",
        path: keyPath("ctrl+shift+t"),
        value: [{ command: "lume.tab.new" }, { command: "lume.tab.close" }],
      },
    ]);
    // Taken from a binding of the user's own, it leaves that binding's key.
    expect(addKey(close, "ctrl+alt+t", "move", held)).toEqual([
      { op: "set", path: keyPath("ctrl+alt+t"), value: "lume.tab.close" },
    ]);
    // What runs only somewhere is unbound first, so the key is this command's there too.
    expect(addKey(command("lume.tab.new"), "ctrl+shift+c", "move", held)).toEqual([
      {
        op: "set",
        path: keyPath("ctrl+shift+c"),
        value: [{ command: "unbound" }, { command: "lume.tab.new" }],
      },
    ]);
  });

  it("unbinds a default in the file, and takes the user's own out of it", () => {
    const held = context();
    const newTab = command("lume.tab.new");
    expect(unbindKey(newTab, newTab.keys[0]!, held)).toEqual([
      { op: "set", path: keyPath("ctrl+shift+t"), value: "unbound" },
    ]);
    expect(unbindKey(newTab, newTab.keys[1]!, held)).toEqual([
      { op: "unset", path: 'keybindings."ctrl+alt+t"' },
    ]);
    expect(restoreKey(command("lume.tab.close").keys[0]!)).toEqual([
      { op: "unset", path: 'keybindings."ctrl+shift+w"' },
    ]);
  });

  it("replaces one key with another", () => {
    const held = context();
    const newTab = command("lume.tab.new");
    expect(replaceKey(newTab, newTab.keys[1]!, "ctrl+alt+y", "keep", held)).toEqual([
      { op: "unset", path: 'keybindings."ctrl+alt+t"' },
      { op: "set", path: keyPath("ctrl+alt+y"), value: "lume.tab.new" },
    ]);
  });

  it("resets one command, or every key, and can be taken back", () => {
    const held = context();
    expect(resetCommand(command("lume.tab.new"), held)).toEqual([
      { op: "unset", path: 'keybindings."ctrl+alt+t"' },
    ]);
    const all = resetAll(held);
    expect(all.map((write) => write.path).sort()).toEqual([
      'keybindings."ctrl+alt+t"',
      'keybindings."ctrl+shift+c"',
      'keybindings."ctrl+shift+w"',
    ]);
    expect(undoOf(all, held)).toEqual(
      expect.arrayContaining([
        { op: "set", path: 'keybindings."ctrl+shift+w"', value: "unbound" },
        { op: "set", path: 'keybindings."ctrl+alt+t"', value: "lume.tab.new" },
      ]),
    );
    expect(undoOf([{ op: "set", path: keyPath("ctrl+alt+q"), value: "x" }], held)).toEqual([
      { op: "unset", path: keyPath("ctrl+alt+q") },
    ]);
  });
});
