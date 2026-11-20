/**
 * The keys of every command as the keybindings page shows them, and the changes to
 * the user's file that changing them asks for.
 *
 * A key is the application's default, a plugin's, or the user's own; a default the
 * user unbound is still shown, struck through, so it can be had back. Two commands on
 * the same keys in the same place share them, and both are marked. What the page asks
 * for is written to `[keybindings]` in the user's file, one key at a time, so the file
 * stays as a person would write it.
 *
 * Nothing here touches the page, so all of it is tested with plain objects.
 */

import { COMMAND_GROUPS, commandGroup, type CommandGroup } from "../commands/groups.ts";
import { chordLabel } from "../commands/store.ts";
import type { CommandSpec, ConfigPlatform, KeyBinding } from "../core/types.ts";
import { joinPath, splitPath } from "./model.ts";

/** What the file says a key does: a command, or `"unbound"` to take its others away. */
export const UNBOUND = "unbound";

/** The table the user's keys are written in. */
const TABLE = "keybindings";

/** The tables of one platform's keys, inside the table of keys. */
const PLATFORM_TABLES: readonly string[] = ["windows", "macos", "linux"];

/** Where a key comes from. */
export type KeySource = "default" | "plugin" | "user" | "workspace";

/** One way to press a command. */
export interface BoundKey {
  /** The chords, as the configuration writes them, one after another: `ctrl+k ctrl+s`. */
  readonly keys: string;
  /** The same written for people: `Ctrl+K Ctrl+S`. */
  readonly label: string;
  readonly source: KeySource;
  /** Whether it is a default the user unbound. */
  readonly off: boolean;
  /** The other commands bound to the same keys in the same place, by id. */
  readonly shared: readonly string[];
  /** Where in the user's file the key is set or unbound, if it is. */
  readonly path?: string;
  /** What the command is given, and where the key holds, as the binding says. */
  readonly args: unknown;
  readonly when: readonly string[];
}

/** One command, with its keys. */
export interface KeyCommand {
  readonly id: string;
  readonly title: string;
  /** What a plugin's command is filed under, which names the plugin. */
  readonly category: string;
  readonly plugin: boolean;
  readonly group: CommandGroup;
  readonly keys: readonly BoundKey[];
  /** The keys the application gives it, as the configuration writes them. */
  readonly defaults: readonly string[];
  /** Whether the user changed its keys: set one, or unbound a default. */
  readonly changed: boolean;
  /** Whether one of its keys is shared with another command. */
  readonly shared: boolean;
}

/** The keys in effect, as the core sends them. */
export interface KeymapView {
  readonly bindings: readonly KeyBinding[];
  /** The bindings that were unbound or replaced, and what did it. */
  readonly removed?: readonly { readonly binding: KeyBinding; readonly by: unknown }[];
}

/** What the page needs of the configuration. */
export interface KeyContext {
  readonly keymap: KeymapView;
  /** Where each setting came from, by its path. */
  readonly sources: Readonly<Record<string, unknown>>;
  /** The keys table of the configuration in use. */
  readonly table: unknown;
  readonly platform: ConfigPlatform;
}

/** A change to the user's file. A key's value may be a list of what it runs. */
export type KeyWrite =
  | { readonly op: "set"; readonly path: string; readonly value: unknown }
  | { readonly op: "unset"; readonly path: string };

type Json = Readonly<Record<string, unknown>>;

function isObject(value: unknown): value is Json {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The layer a source names, and the file and line it was written at. */
function layerOf(source: unknown): { layer: string; file?: string; line?: number } {
  if (!isObject(source)) return { layer: "default" };
  const layer = typeof source["layer"] === "string" ? source["layer"] : "default";
  return {
    layer,
    ...(typeof source["file"] === "string" ? { file: source["file"] } : {}),
    ...(typeof source["line"] === "number" ? { line: source["line"] } : {}),
  };
}

function kindOf(source: unknown): KeySource {
  const { layer } = layerOf(source);
  if (layer === "user" || layer === "workspace" || layer === "plugin") return layer;
  return "default";
}

/** The chords of a binding joined as the configuration writes a sequence. */
function keysOf(binding: KeyBinding): string {
  return binding.keys.join(" ");
}

/** A sequence of chords written for people. */
export function keysLabel(keys: string, platform: ConfigPlatform): string {
  return keys
    .split(" ")
    .filter((chord) => chord !== "")
    .map((chord) => chordLabel(chord, platform))
    .join(" ");
}

/** The path a key is written at in the table of every platform. */
export function keyPath(keys: string): string {
  return joinPath([TABLE, keys]);
}

/** Whether a path is one of the user's keys, in the table of every platform or of one. */
function isKeyPath(path: string): boolean {
  const parts = splitPath(path);
  if (parts[0] !== TABLE) return false;
  if (parts.length === 2) return !PLATFORM_TABLES.includes(parts[1] ?? "");
  return parts.length === 3 && PLATFORM_TABLES.includes(parts[1] ?? "");
}

/** The path of the user's file that a binding written there was read from. */
function pathOf(source: unknown, sources: KeyContext["sources"]): string | undefined {
  const { layer, file, line } = layerOf(source);
  if (layer !== "user" || file === undefined || line === undefined) return undefined;
  for (const [path, held] of Object.entries(sources)) {
    if (!isKeyPath(path)) continue;
    const other = layerOf(held);
    if (other.layer === "user" && other.file === file && other.line === line) return path;
  }
  return undefined;
}

/** What the configuration in use says at a path of the keys table. */
function valueAt(table: unknown, path: string): unknown {
  let at: unknown = table;
  for (const part of splitPath(path).slice(1)) {
    if (!isObject(at)) return undefined;
    at = at[part];
  }
  return at;
}

/** Whether two bindings hold in the same place: one of them everywhere, or both alike. */
function overlap(one: readonly string[], other: readonly string[]): boolean {
  if (one.length === 0 || other.length === 0) return true;
  return one.length === other.length && one.every((term) => other.includes(term));
}

/**
 * Every command with its keys, gathered as the list of commands is: by what it acts
 * on, each group's by name.
 */
export function keyCommands(commands: readonly CommandSpec[], context: KeyContext): KeyCommand[] {
  const { keymap, sources, table, platform } = context;
  const active = keymap.bindings;
  const removed = keymap.removed ?? [];

  const listed = commands.map((spec): KeyCommand => {
    const keys: BoundKey[] = [];
    const add = (key: BoundKey) => {
      // Bindings that give the command different things are ways to press it all the
      // same; two written alike are one.
      if (!keys.some((held) => held.keys === key.keys && held.off === key.off)) keys.push(key);
    };
    for (const binding of active) {
      if (binding.command !== spec.id) continue;
      const written = keysOf(binding);
      const shared = active
        .filter(
          (other) =>
            other.command !== spec.id &&
            keysOf(other) === written &&
            overlap(other.when, binding.when),
        )
        .map((other) => other.command);
      const path = pathOf(binding.source, sources);
      add({
        keys: written,
        label: keysLabel(written, platform),
        source: kindOf(binding.source),
        off: false,
        shared: [...new Set(shared)],
        ...(path === undefined ? {} : { path }),
        args: binding.args,
        when: binding.when,
      });
    }
    for (const { binding, by } of removed) {
      if (binding.command !== spec.id || kindOf(binding.source) !== "default") continue;
      const written = keysOf(binding);
      const path = pathOf(by, sources);
      // A default that gave way to another binding on its key shows with that one; a
      // default the user unbound shows here, struck through.
      if (path === undefined || valueAt(table, path) !== UNBOUND) continue;
      if (keys.some((held) => held.keys === written)) continue;
      add({
        keys: written,
        label: keysLabel(written, platform),
        source: "default",
        off: true,
        shared: [],
        path,
        args: binding.args,
        when: binding.when,
      });
    }
    const defaults = [
      ...new Set([
        ...active
          .filter((binding) => binding.command === spec.id && kindOf(binding.source) === "default")
          .map(keysOf),
        ...removed
          .filter(
            ({ binding }) => binding.command === spec.id && kindOf(binding.source) === "default",
          )
          .map(({ binding }) => keysOf(binding)),
      ]),
    ];
    return {
      id: spec.id,
      title: spec.title,
      category: spec.category,
      plugin: !spec.id.startsWith("lume."),
      group: commandGroup(spec.id),
      keys,
      defaults,
      changed: keys.some((key) => key.off || key.source === "user"),
      shared: keys.some((key) => !key.off && key.shared.length > 0),
    };
  });

  return COMMAND_GROUPS.flatMap((group) =>
    listed
      .filter((command) => command.group === group)
      .sort((one, other) => one.title.localeCompare(other.title) || one.id.localeCompare(other.id)),
  );
}

/** The ways the list can be narrowed, beside what is typed. */
export type KeyFilter = "all" | "changed" | "off" | "shared" | "none";

/** What each filter keeps. */
export const KEY_FILTERS: readonly { readonly id: KeyFilter; readonly label: string }[] = [
  { id: "all", label: "All" },
  { id: "changed", label: "Changed" },
  { id: "off", label: "Unbound defaults" },
  { id: "shared", label: "Shared keys" },
  { id: "none", label: "No keys" },
];

/** Whether a command is one a filter keeps. */
export function keptBy(filter: KeyFilter, command: KeyCommand): boolean {
  switch (filter) {
    case "all":
      return true;
    case "changed":
      return command.changed;
    case "off":
      return command.keys.some((key) => key.off);
    case "shared":
      return command.shared;
    case "none":
      return !command.keys.some((key) => !key.off);
  }
}

/**
 * Whether every word typed is somewhere in a command: its name, id, group or keys, as
 * written for people or as the configuration writes them.
 */
export function keyMatches(command: KeyCommand, query: string): boolean {
  const words = query
    .toLowerCase()
    .split(/\s+/)
    .filter((word) => word !== "");
  if (words.length === 0) return true;
  const hay = [
    command.title,
    command.id,
    command.category,
    command.group,
    ...command.keys.flatMap((key) => [key.label, key.keys]),
  ]
    .join(" ")
    .toLowerCase();
  return words.every((word) => hay.includes(word));
}

/** One entry of what a key runs, as the file writes it. */
interface Entry {
  readonly command: string;
  readonly when?: string;
  readonly args?: unknown;
}

/** What a binding is written as in a list of what a key runs. */
function entryOf(binding: { command: string; args: unknown; when: readonly string[] }): Entry {
  return {
    command: binding.command,
    ...(binding.when.length === 0 ? {} : { when: binding.when.join(" && ") }),
    ...(binding.args === null || binding.args === undefined ? {} : { args: binding.args }),
  };
}

/** What a key's value in the file comes to, as entries. */
function entriesOf(value: unknown): Entry[] {
  if (typeof value === "string") return [{ command: value }];
  if (isObject(value) && typeof value["command"] === "string") {
    const when = value["when"];
    return [
      {
        command: value["command"],
        ...(typeof when === "string" ? { when } : {}),
        ...(value["args"] === undefined ? {} : { args: value["args"] }),
      },
    ];
  }
  if (Array.isArray(value)) return value.flatMap((item: unknown) => entriesOf(item));
  return [];
}

/** Entries as a key's value: a lone command as its id, as a person writes it. */
function valueOf(entries: readonly Entry[]): unknown {
  const [only] = entries;
  if (entries.length === 1 && only && only.when === undefined && only.args === undefined) {
    return only.command;
  }
  return entries;
}

/** What the command gives a new key of its own: the args of its keys, if they all agree. */
function argsFor(command: KeyCommand): unknown {
  const given = new Set(command.keys.map((key) => JSON.stringify(key.args ?? null)));
  if (given.size !== 1) return null;
  const [only] = command.keys;
  return only?.args ?? null;
}

/** How a key taken by other commands is given to a command. */
export type Taking = "keep" | "move";

/**
 * The changes that bind `keys` to a command. A default of the command that was
 * unbound is had back instead. Other commands on the same keys keep them alongside
 * (`keep`), or lose them to this one (`move`).
 */
export function addKey(
  command: KeyCommand,
  keys: string,
  taking: Taking,
  context: KeyContext,
): KeyWrite[] {
  const off = command.keys.find((key) => key.off && key.keys === keys);
  if (off?.path !== undefined && taking === "keep") return [{ op: "unset", path: off.path }];
  if (command.keys.some((key) => !key.off && key.keys === keys)) return [];

  const path = keyPath(keys);
  const mine: Entry = entryOf({ command: command.id, args: argsFor(command), when: [] });
  // What the key runs now that a binding written everywhere would take away, and what
  // it runs only somewhere, which it would not.
  const theirs = context.keymap.bindings.filter(
    (binding) => keysOf(binding) === keys && binding.command !== command.id,
  );
  const writes: KeyWrite[] = [];
  // A binding of the same keys written elsewhere in the file gives way to this one.
  for (const binding of theirs) {
    const elsewhere = pathOf(binding.source, context.sources);
    if (elsewhere !== undefined && elsewhere !== path && taking === "move") {
      writes.push(...withoutKey(elsewhere, binding.command, context));
    }
  }
  if (taking === "keep") {
    const kept = theirs.filter((binding) => binding.when.length === 0).map(entryOf);
    writes.push({ op: "set", path, value: valueOf([...kept, mine]) });
  } else {
    const somewhere = theirs.some((binding) => binding.when.length > 0);
    writes.push({
      op: "set",
      path,
      value: valueOf(somewhere ? [{ command: UNBOUND }, mine] : [mine]),
    });
  }
  // A default of these keys the command had, unbound before, is its own again.
  if (off?.path !== undefined && off.path !== path) writes.unshift({ op: "unset", path: off.path });
  return writes;
}

/** The changes that take one command's entries out of the key written at `path`. */
function withoutKey(path: string, id: string, context: KeyContext): KeyWrite[] {
  const left = entriesOf(valueAt(context.table, path)).filter((entry) => entry.command !== id);
  if (left.length === 0) return [{ op: "unset", path }];
  return [{ op: "set", path, value: valueOf(left) }];
}

/** The changes that take a key away from a command. */
export function unbindKey(command: KeyCommand, key: BoundKey, context: KeyContext): KeyWrite[] {
  if (key.off) return [];
  if (key.source === "user" && key.path !== undefined) {
    return withoutKey(key.path, command.id, context);
  }
  return [{ op: "set", path: keyPath(key.keys), value: UNBOUND }];
}

/** The changes that give a command back a default it was unbound from. */
export function restoreKey(key: BoundKey): KeyWrite[] {
  return key.off && key.path !== undefined ? [{ op: "unset", path: key.path }] : [];
}

/** The changes that put another key where one of a command's is. */
export function replaceKey(
  command: KeyCommand,
  key: BoundKey,
  keys: string,
  taking: Taking,
  context: KeyContext,
): KeyWrite[] {
  if (key.keys === keys) return [];
  const rest = { ...command, keys: command.keys.filter((held) => held !== key) };
  return [...unbindKey(command, key, context), ...addKey(rest, keys, taking, context)];
}

/** The changes that give a command its default keys and nothing else. */
export function resetCommand(command: KeyCommand, context: KeyContext): KeyWrite[] {
  const writes: KeyWrite[] = [];
  for (const key of command.keys) {
    if (key.path === undefined) continue;
    if (key.off) writes.push({ op: "unset", path: key.path });
    else if (key.source === "user") writes.push(...withoutKey(key.path, command.id, context));
  }
  return writes;
}

/** The changes that take every key the user set or unbound out of their file. */
export function resetAll(context: KeyContext): KeyWrite[] {
  return Object.entries(context.sources)
    .filter(([path, source]) => isKeyPath(path) && layerOf(source).layer === "user")
    .map(([path]) => ({ op: "unset", path }));
}

/**
 * The changes that put the user's file back as it was before `writes`, from the
 * configuration as it was then: what each path held, or nothing.
 */
export function undoOf(writes: readonly KeyWrite[], context: KeyContext): KeyWrite[] {
  const paths = [...new Set(writes.map((write) => write.path))];
  return paths.map((path) => {
    const held =
      layerOf(context.sources[path]).layer === "user" ? valueAt(context.table, path) : undefined;
    return held === undefined ? { op: "unset", path } : { op: "set", path, value: held };
  });
}

/** The commands other than `id` that run `keys` now, by name. */
export function othersOn(
  commands: readonly KeyCommand[],
  id: string,
  keys: string,
): readonly KeyCommand[] {
  return commands.filter(
    (command) => command.id !== id && command.keys.some((key) => !key.off && key.keys === keys),
  );
}
