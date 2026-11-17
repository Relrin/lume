/**
 * What the Connect palette lists, worked out from the profiles there are and what was
 * typed: groups of profiles by where their terminal is, the actions after them, and
 * what is said of the profile that is marked.
 *
 * Nothing here touches the page, so all of it is tested with plain objects.
 */

import type { Profile, ProfileList } from "../core/types.ts";
import { rank } from "../palette/fuzzy.ts";
import type { Launcher } from "../transport/types.ts";
import { CONNECT_GROUP_MAX } from "./tuning.ts";

/** Where a profile's terminal is. */
export type ConnectKind = "local" | "wsl" | "ssh";

/** Which kinds of profile the palette lists. */
export type ConnectFilter = "all" | ConnectKind;

/** The filters, in the order the palette offers them, with what each is called. */
export const CONNECT_FILTERS: readonly { readonly value: ConnectFilter; readonly label: string }[] =
  [
    { value: "all", label: "All" },
    { value: "ssh", label: "SSH" },
    { value: "wsl", label: "WSL" },
    { value: "local", label: "Local" },
  ];

/** The groups, in the order they are listed, with their headings. */
const GROUPS: readonly { readonly kind: ConnectKind; readonly label: string }[] = [
  { kind: "local", label: "Local shells" },
  { kind: "wsl", label: "WSL distros" },
  { kind: "ssh", label: "SSH hosts" },
];

/** One profile as the palette lists it. */
export interface ConnectEntry {
  readonly profile: Profile;
  readonly kind: ConnectKind;
  /** Whether new terminals use it when they name none. */
  readonly isDefault: boolean;
  /** Which characters of the profile's name the query matched. */
  readonly indices: readonly number[];
  /** The line under the name: where the terminal goes, or what it runs. */
  readonly line: string;
}

/** The profiles of one kind. */
export interface ConnectGroup {
  readonly kind: ConnectKind;
  readonly label: string;
  readonly entries: readonly ConnectEntry[];
}

/** Something the palette does other than open a terminal. */
export interface ConnectAction {
  readonly id: string;
  readonly label: string;
  /** The command it runs. */
  readonly command: string;
  /** What is said of it beside the list while it is marked. */
  readonly about: string;
}

/** The actions the palette offers after the profiles. */
export const CONNECT_ACTIONS: readonly ConnectAction[] = [
  {
    id: "config",
    label: "Open config.toml",
    command: "lume.config.open",
    about: "Profiles of your own are written as [[profile]] tables in config.toml.",
  },
  {
    id: "refresh",
    label: "Look for profiles again",
    command: "lume.profile.refresh",
    about: "Looks again for shells, Linux distributions and the hosts of your SSH config.",
  },
];

/** One row of the list the keyboard moves through. */
export type ConnectRow =
  | { readonly type: "profile"; readonly entry: ConnectEntry }
  | { readonly type: "action"; readonly action: ConnectAction };

/** Where a profile's terminal is. */
export function kindOf(profile: Pick<Profile, "launcher">): ConnectKind {
  switch (profile.launcher.type) {
    case "wsl":
      return "wsl";
    case "ssh":
      return "ssh";
    default:
      return "local";
  }
}

/** A program's name without its folder. */
function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter((part) => part !== "");
  return parts.at(-1) ?? path;
}

/** An argument as it would be typed: quoted when it has a blank in it, or is empty. */
function typed(argument: string): string {
  return argument === "" || /\s/.test(argument) ? `"${argument.replace(/"/g, '\\"')}"` : argument;
}

/** Where an SSH profile connects to, as a person writes it: `user@host:port`. */
function target(launcher: Extract<Launcher, { type: "ssh" }>): string {
  const user = launcher.user === undefined ? "" : `${launcher.user}@`;
  const port = launcher.port === undefined ? "" : `:${launcher.port}`;
  return `${user}${launcher.host}${port}`;
}

/** What starting a profile runs, as a command line a person can read. */
export function describeRun(profile: Pick<Profile, "launcher">): string {
  const launcher = profile.launcher;
  switch (launcher.type) {
    case "shell": {
      const program = launcher.program.trim() === "" ? "your own shell" : typed(launcher.program);
      return [program, ...launcher.args.map(typed)].join(" ");
    }
    case "wsl":
      return [
        "wsl.exe",
        "-d",
        typed(launcher.distro),
        ...(launcher.user ? ["-u", typed(launcher.user)] : []),
      ].join(" ");
    case "ssh": {
      const parts = [launcher.client === undefined ? "ssh" : typed(launcher.client)];
      parts.push(...(launcher.extra_args ?? []).map(typed));
      if (launcher.port !== undefined) parts.push("-p", String(launcher.port));
      parts.push(
        typed(`${launcher.user === undefined ? "" : `${launcher.user}@`}${launcher.host}`),
      );
      return parts.join(" ");
    }
  }
}

/** The line under a profile's name. */
export function lineOf(profile: Profile): string {
  const launcher = profile.launcher;
  switch (launcher.type) {
    case "ssh":
      return target(launcher);
    case "wsl":
      return profile.detail ?? `WSL · ${launcher.distro}`;
    case "shell":
      if (profile.detail !== undefined) return profile.detail;
      return launcher.program.trim() === "" ? "Your own shell" : baseName(launcher.program);
  }
}

/** Where a profile comes from, as the palette says it. */
export function sourceOf(profile: Profile): string {
  if (profile.origin === "user") return "Your profile, in config.toml";
  switch (kindOf(profile)) {
    case "ssh":
      return "A host of your SSH config";
    case "wsl":
      return "A Linux distribution on this machine";
    case "local":
      return "A shell found on this machine";
  }
}

/** Whether a profile is one the palette lists under a filter. */
function passes(profile: Profile, filter: ConnectFilter): boolean {
  return !profile.hidden && (filter === "all" || kindOf(profile) === filter);
}

/** What a profile is found by besides its name. */
function otherText(profile: Profile): string {
  return [profile.id, profile.detail ?? "", lineOf(profile), describeRun(profile)].join(" ");
}

/**
 * The profiles to list, by kind. Without a query a group lists the user's own profiles
 * first and then the ones that were found, each in the order the list gives them. With
 * one, it lists the profiles whose name matches, best first, and after them those that
 * match by their id or what they run. A group with nothing in it is left out.
 */
export function connectGroups(
  list: Pick<ProfileList, "profiles" | "default"> | null,
  query: string,
  filter: ConnectFilter,
): ConnectGroup[] {
  if (!list) return [];
  const offered = list.profiles.filter((profile) => passes(profile, filter));
  const entry = (profile: Profile, indices: readonly number[]): ConnectEntry => ({
    profile,
    kind: kindOf(profile),
    isDefault: list.default !== null && profile.id === list.default,
    indices,
    line: lineOf(profile),
  });
  return GROUPS.flatMap(({ kind, label }) => {
    const own = offered.filter((profile) => kindOf(profile) === kind);
    let entries: ConnectEntry[];
    if (query.trim() === "") {
      const mine = own.filter((profile) => profile.origin === "user");
      const found = own.filter((profile) => profile.origin !== "user");
      entries = [...mine, ...found].map((profile) => entry(profile, []));
    } else {
      const byName = rank(own, query, (profile) => profile.name);
      const named = new Set(byName.map((match) => match.item));
      const byOther = rank(
        own.filter((profile) => !named.has(profile)),
        query,
        otherText,
      );
      entries = [
        ...byName.map((match) => entry(match.item, match.indices)),
        ...byOther.map((match) => entry(match.item, [])),
      ];
    }
    entries = entries.slice(0, CONNECT_GROUP_MAX);
    return entries.length === 0 ? [] : [{ kind, label, entries }];
  });
}

/** The rows the keyboard moves through: every profile listed, then the actions. */
export function connectRows(groups: readonly ConnectGroup[]): ConnectRow[] {
  return [
    ...groups.flatMap((group) =>
      group.entries.map((entry): ConnectRow => ({ type: "profile", entry })),
    ),
    ...CONNECT_ACTIONS.map((action): ConnectRow => ({ type: "action", action })),
  ];
}

/** The filter after `filter` when the palette's filters are stepped through, or before it. */
export function stepFilter(filter: ConnectFilter, by: 1 | -1): ConnectFilter {
  const at = CONNECT_FILTERS.findIndex((candidate) => candidate.value === filter);
  const next = (at + by + CONNECT_FILTERS.length) % CONNECT_FILTERS.length;
  return CONNECT_FILTERS[next]?.value ?? "all";
}

/** What opening a profile asks the core for: a new tab, or a split beside the focused pane. */
export function openCommand(
  profile: string,
  how: "tab" | "split",
  hasFocus: boolean,
): { readonly id: string; readonly args: { readonly profile: string } } {
  if (how === "split" && hasFocus) return { id: "lume.pane.split-right", args: { profile } };
  return {
    id: how === "tab" ? "lume.terminal.new-with-profile" : "lume.tab.new",
    args: { profile },
  };
}
