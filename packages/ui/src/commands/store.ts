/**
 * The commands there are, and the keys that run them written for people.
 *
 * The core keeps the list of commands: each has an id, a title, and says whether it is
 * carried out in the core or in the page. A page takes the list once and looks
 * commands up in it by id.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { CommandSpec, ConfigPlatform, KeyBinding } from "../core/types.ts";

/** The commands a page knows of. */
export interface CommandState {
  /** The commands in the order the core lists them. */
  readonly commands: readonly CommandSpec[];
  /** The same commands by id. */
  readonly byId: ReadonlyMap<string, CommandSpec>;
  /** Replaces the list. */
  set(commands: readonly CommandSpec[]): void;
  /** The command with this id, if there is one. */
  find(id: string): CommandSpec | undefined;
}

/** The store a page keeps its commands in. */
export type CommandStore = StoreApi<CommandState>;

/** Makes a page's command store. It is empty until it is given the core's list. */
export function createCommandStore(): CommandStore {
  return createStore<CommandState>()((set, get) => ({
    commands: [],
    byId: new Map(),
    set: (commands) =>
      set({ commands, byId: new Map(commands.map((command) => [command.id, command])) }),
    find: (id) => get().byId.get(id),
  }));
}

/** How a platform writes its modifiers, in the order it writes them, and what joins them. */
interface Notation {
  readonly ctrl: string;
  readonly alt: string;
  readonly shift: string;
  readonly logo: string;
  readonly join: string;
}

const NOTATIONS: Readonly<Record<ConfigPlatform, Notation>> = {
  windows: { ctrl: "Ctrl", alt: "Alt", shift: "Shift", logo: "Win", join: "+" },
  linux: { ctrl: "Ctrl", alt: "Alt", shift: "Shift", logo: "Super", join: "+" },
  macos: { ctrl: "⌃", alt: "⌥", shift: "⇧", logo: "⌘", join: "" },
};

/** The names of keys as people read them, where that is not the key's own spelling. */
const KEY_LABELS: ReadonlyMap<string, string> = new Map([
  ["enter", "Enter"],
  ["tab", "Tab"],
  ["space", "Space"],
  ["escape", "Esc"],
  ["backspace", "Backspace"],
  ["delete", "Delete"],
  ["insert", "Insert"],
  ["home", "Home"],
  ["end", "End"],
  ["pageup", "PageUp"],
  ["pagedown", "PageDown"],
  ["up", "↑"],
  ["down", "↓"],
  ["left", "←"],
  ["right", "→"],
  ["contextmenu", "Menu"],
]);

const MODIFIERS: ReadonlyMap<string, keyof Omit<Notation, "join">> = new Map([
  ["ctrl", "ctrl"],
  ["alt", "alt"],
  ["shift", "shift"],
  ["super", "logo"],
]);

/** One chord, such as `ctrl+shift+t`, written for people: `Ctrl+Shift+T`, or `⇧⌘T` on a Mac. */
export function chordLabel(chord: string, platform: ConfigPlatform): string {
  const notation = NOTATIONS[platform];
  // The key that types `+` is written as itself after the `+` that joins.
  const cut = chord.endsWith("+") ? chord.length - 1 : chord.lastIndexOf("+") + 1;
  const key = chord.slice(cut);
  const held = chord
    .slice(0, Math.max(0, cut - 1))
    .split("+")
    .filter((part) => part !== "");
  const parts = held.map((modifier) => {
    const name = MODIFIERS.get(modifier);
    return name ? notation[name] : modifier;
  });
  // A character is written as a capital, and so is a function key's name.
  parts.push(KEY_LABELS.get(key) ?? key.toUpperCase());
  return parts.join(notation.join);
}

/**
 * The keys that run a command, written for people, or nothing when no key does. It is
 * the first binding of the command, which among the defaults is the usual one; the
 * chords of a sequence are separated by spaces. With `args`, only a binding that gives
 * the command exactly those is looked for.
 */
export function keyLabel(
  bindings: readonly KeyBinding[],
  command: string,
  platform: ConfigPlatform,
  args?: unknown,
): string | undefined {
  const wanted = args === undefined ? undefined : JSON.stringify(args);
  const binding = bindings.find(
    (candidate) =>
      candidate.command === command &&
      (wanted === undefined || JSON.stringify(candidate.args) === wanted),
  );
  return binding?.keys.map((chord) => chordLabel(chord, platform)).join(" ");
}
