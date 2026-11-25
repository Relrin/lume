/**
 * The commands as a person looks them up: every one there is, gathered under what it
 * acts on, with the keys that run it now.
 *
 * A command's id says what it acts on: `lume.pane.split-right` acts on a pane. The
 * ids' areas are more than a list is easy to read by, so they are gathered into six
 * groups here, and a plugin's commands go with the application's. Nothing here touches
 * the page, so all of it is tested with plain objects.
 */

import type { CommandSpec, ConfigPlatform, KeyBinding } from "../core/types.ts";
import { lookup } from "../lookup.ts";
import { chordLabel } from "./store.ts";

/** What a command acts on, in the order the groups are listed. */
export const COMMAND_GROUPS = ["Session", "Tab", "Pane", "Host", "View", "App"] as const;

/** One of the groups commands are listed under. */
export type CommandGroup = (typeof COMMAND_GROUPS)[number];

/** The group of each area of the application's own ids. */
const GROUP_OF_AREA: Readonly<Record<string, CommandGroup>> = {
  session: "Session",
  connect: "Session",
  workspace: "Session",
  tab: "Tab",
  group: "Tab",
  pane: "Pane",
  terminal: "Pane",
  search: "Pane",
  host: "Host",
  rex: "Host",
  profile: "Host",
  view: "View",
  panel: "View",
  window: "View",
  palette: "View",
  app: "App",
  config: "App",
  settings: "App",
};

/** The commands whose id says one thing and which act on another. */
const GROUP_OF_COMMAND: Readonly<Record<string, CommandGroup>> = {
  // It opens a tab; the terminal in it is only what the tab shows.
  "lume.terminal.new-with-profile": "Tab",
};

/** The group a command is listed under. A plugin's command is the application's to list. */
export function commandGroup(id: string): CommandGroup {
  const own = lookup(GROUP_OF_COMMAND, id);
  if (own) return own;
  const [owner, area] = id.split(".");
  if (owner !== "lume" || area === undefined) return "App";
  return lookup(GROUP_OF_AREA, area) ?? "App";
}

/** One command of the list: what it is called, and the keys that run it. */
export interface ListedCommand {
  readonly id: string;
  readonly title: string;
  /** What the application files it under, which for a plugin's command names the plugin. */
  readonly category: string;
  /** Whether a plugin brings it. */
  readonly plugin: boolean;
  /** Every way to press it, written for people, the usual one first. */
  readonly keys: readonly string[];
}

/** The commands of one group, in the order they are read. */
export interface ListedGroup {
  readonly group: CommandGroup;
  readonly commands: readonly ListedCommand[];
}

/**
 * Every way to press a command, written for people. Bindings that hand the command
 * different things, as the keys that go to the first tab and to the second do, are all
 * ways to press it; two that are written alike are one.
 */
export function keysOf(
  bindings: readonly KeyBinding[],
  command: string,
  platform: ConfigPlatform,
): string[] {
  const keys: string[] = [];
  for (const binding of bindings) {
    if (binding.command !== command) continue;
    const label = binding.keys.map((chord) => chordLabel(chord, platform)).join(" ");
    if (!keys.includes(label)) keys.push(label);
  }
  return keys;
}

/** Whether every word of what was typed is somewhere in what a command is. */
function matches(command: ListedCommand, group: CommandGroup, words: readonly string[]): boolean {
  const hay = [command.title, command.id, command.category, group, ...command.keys]
    .join(" ")
    .toLowerCase();
  return words.every((word) => hay.includes(word));
}

/**
 * The commands gathered into their groups, each group's by name, with the keys bound
 * to them now. With `query`, only the commands it finds; a group left with none is
 * left out.
 */
export function listCommands(
  commands: readonly CommandSpec[],
  bindings: readonly KeyBinding[],
  platform: ConfigPlatform,
  query = "",
): ListedGroup[] {
  const words = query
    .toLowerCase()
    .split(/\s+/)
    .filter((word) => word !== "");
  const groups = new Map<CommandGroup, ListedCommand[]>();
  for (const spec of commands) {
    const group = commandGroup(spec.id);
    const listed: ListedCommand = {
      id: spec.id,
      title: spec.title,
      category: spec.category,
      plugin: !spec.id.startsWith("lume."),
      keys: keysOf(bindings, spec.id, platform),
    };
    if (!matches(listed, group, words)) continue;
    const held = groups.get(group);
    if (held) held.push(listed);
    else groups.set(group, [listed]);
  }
  return COMMAND_GROUPS.flatMap((group) => {
    const held = groups.get(group);
    if (!held) return [];
    held.sort(
      (one, other) => one.title.localeCompare(other.title) || one.id.localeCompare(other.id),
    );
    return [{ group, commands: held }];
  });
}
