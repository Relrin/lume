/**
 * What the Connect palette lists, worked out from the profiles and the Rex servers there
 * are and what was typed: the servers, groups of profiles by where their terminal is,
 * the sessions that run with no pane to show them, the actions after them, and what is
 * said of the row that is marked.
 *
 * Nothing here touches the page, so all of it is tested with plain objects.
 */

import type { DetachedSession, Profile, ProfileList, RexHost, RexHosts } from "../core/types.ts";
import { rank } from "../palette/fuzzy.ts";
import { nearness, sessionName, standing } from "../rex/model.ts";
import type { Launcher } from "../transport/types.ts";
import { CONNECT_GROUP_MAX } from "./tuning.ts";

/** Where a profile's terminal is. */
export type ConnectKind = "local" | "wsl" | "ssh";

/** Which kinds of profile the palette lists, or the Rex servers, or the detached sessions. */
export type ConnectFilter = "all" | "rex" | ConnectKind | "detached";

/** The filters, in the order the palette offers them, with what each is called. */
export const CONNECT_FILTERS: readonly { readonly value: ConnectFilter; readonly label: string }[] =
  [
    { value: "all", label: "All" },
    { value: "rex", label: "Rex" },
    { value: "ssh", label: "SSH" },
    { value: "wsl", label: "WSL" },
    { value: "local", label: "Local" },
    { value: "detached", label: "Detached" },
  ];

/**
 * The filters the palette offers now. "Detached" is offered while there are detached
 * sessions, or while it is the filter in use, so that it is not a choice that always
 * lists nothing.
 */
export function connectFilters(
  hasDetached: boolean,
  current: ConnectFilter,
): readonly { readonly value: ConnectFilter; readonly label: string }[] {
  return CONNECT_FILTERS.filter(
    (choice) => choice.value !== "detached" || hasDetached || current === "detached",
  );
}

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
  /** Whether the user pinned it, so it is listed first. */
  readonly pinned: boolean;
}

/** The heading of the group of Rex servers. */
export const REX_LABEL = "Rex servers";

/** One Rex server as the palette lists it. */
export interface HostEntry {
  readonly host: RexHost;
  /** Which characters of the server's name the query matched. */
  readonly indices: readonly number[];
  /** The line after the name: how many sessions it has, or why they are not known. */
  readonly line: string;
  /** How near it is, at the row's end. */
  readonly meta: string;
  /** Whether the user pinned it, so it is listed first. */
  readonly pinned: boolean;
}

/** The profiles of one kind, or those pinned. */
export interface ConnectGroup {
  readonly kind: ConnectKind | "pinned";
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

/** The heading of the group of detached sessions. */
export const DETACHED_LABEL = "Detached sessions";

/** One detached session as the palette lists it. */
export interface DetachedEntry {
  readonly session: DetachedSession;
  /** What the palette calls it: the title its program gave it, or the program. */
  readonly name: string;
  /** Which characters of the name the query matched. */
  readonly indices: readonly number[];
  /** The line under the name: where it is, whether it runs, and when it started. */
  readonly line: string;
}

/** One row of the list the keyboard moves through. */
export type ConnectRow =
  | { readonly type: "host"; readonly entry: HostEntry }
  | { readonly type: "profile"; readonly entry: ConnectEntry }
  | { readonly type: "session"; readonly entry: DetachedEntry }
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
 *
 * The profiles in `pinned` are taken out of their kinds' groups and listed first, in a
 * group of their own, in the order their groups had them.
 */
export function connectGroups(
  list: Pick<ProfileList, "profiles" | "default"> | null,
  query: string,
  filter: ConnectFilter,
  pinned: ReadonlySet<string> = new Set(),
): ConnectGroup[] {
  if (!list) return [];
  const offered = list.profiles.filter((profile) => passes(profile, filter));
  const entry = (profile: Profile, indices: readonly number[]): ConnectEntry => ({
    profile,
    kind: kindOf(profile),
    isDefault: list.default !== null && profile.id === list.default,
    indices,
    line: lineOf(profile),
    pinned: pinned.has(profile.id),
  });
  const groups: ConnectGroup[] = GROUPS.flatMap(({ kind, label }) => {
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
  if (pinned.size === 0) return groups;
  const first = groups.flatMap((group) => group.entries.filter((entry) => entry.pinned));
  const rest = groups
    .map((group) => ({ ...group, entries: group.entries.filter((entry) => !entry.pinned) }))
    .filter((group) => group.entries.length > 0);
  return first.length === 0 ? rest : [{ kind: "pinned", label: "Pinned", entries: first }, ...rest];
}

/** How long ago something started, as the palette says it: "started 5m ago". */
export function startedAgo(startedMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.floor((nowMs - startedMs) / 1000));
  if (seconds < 60) return "started just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `started ${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `started ${hours}h ago`;
  return `started ${Math.floor(hours / 24)}d ago`;
}

/** What a detached session is called in the palette. */
export function detachedName(session: DetachedSession): string {
  const title = session.title?.trim() ?? "";
  return title === "" ? baseName(session.program) : title;
}

/**
 * The detached sessions to list. Without a query, in the order they are given; with
 * one, those whose name matches, best first, then those that match by their program or
 * directory. None under a filter of one kind of profile, and none before they are known.
 * `place` says a directory the way the page shows it.
 */
export function detachedEntries(
  sessions: readonly DetachedSession[] | null,
  query: string,
  filter: ConnectFilter,
  nowMs: number,
  place: (cwd: string) => string = (cwd) => cwd,
): DetachedEntry[] {
  if (!sessions || (filter !== "all" && filter !== "detached")) return [];
  const entry = (session: DetachedSession, indices: readonly number[]): DetachedEntry => ({
    session,
    name: detachedName(session),
    indices,
    line: [
      session.cwd === null || session.cwd === "" ? null : place(session.cwd),
      session.exited ? "ended" : "running",
      startedAgo(session.created_unix_ms, nowMs),
    ]
      .filter((part): part is string => part !== null)
      .join(" · "),
  });
  let entries: DetachedEntry[];
  if (query.trim() === "") {
    entries = sessions.map((session) => entry(session, []));
  } else {
    const byName = rank(sessions, query, detachedName);
    const named = new Set(byName.map((match) => match.item));
    const byOther = rank(
      sessions.filter((session) => !named.has(session)),
      query,
      (session) => [session.program, session.cwd ?? ""].join(" "),
    );
    entries = [
      ...byName.map((match) => entry(match.item, match.indices)),
      ...byOther.map((match) => entry(match.item, [])),
    ];
  }
  return entries.slice(0, CONNECT_GROUP_MAX);
}

/**
 * The Rex servers to list. Without a query, in the order they are given; with one,
 * those whose name matches, best first, then those that match by their address or by
 * what a session of theirs is called. None under a filter of another kind.
 */
export function hostEntries(
  list: Pick<RexHosts, "hosts"> | null,
  query: string,
  filter: ConnectFilter,
  pinned: ReadonlySet<string> = new Set(),
): HostEntry[] {
  if (!list || (filter !== "all" && filter !== "rex")) return [];
  const entry = (host: RexHost, indices: readonly number[]): HostEntry => ({
    host,
    indices,
    line: standing(host),
    meta: nearness(host),
    pinned: pinned.has(host.id),
  });
  let entries: HostEntry[];
  if (query.trim() === "") {
    entries = list.hosts.map((host) => entry(host, []));
  } else {
    const byName = rank(list.hosts, query, (host) => host.label);
    const named = new Set(byName.map((match) => match.item));
    const byOther = rank(
      list.hosts.filter((host) => !named.has(host)),
      query,
      (host) => [host.address, ...host.sessions.map(sessionName)].join(" "),
    );
    entries = [
      ...byName.map((match) => entry(match.item, match.indices)),
      ...byOther.map((match) => entry(match.item, [])),
    ];
  }
  return entries.slice(0, CONNECT_GROUP_MAX);
}

/** Rows that stand under one heading. */
export interface ConnectSection {
  readonly key: string;
  readonly label: string;
  readonly rows: readonly ConnectRow[];
}

/**
 * The list as it stands on the page: what is pinned, the Rex servers, the profiles by
 * kind, the detached sessions, and the actions.
 */
export function connectSections(
  groups: readonly ConnectGroup[],
  detached: readonly DetachedEntry[] = [],
  hosts: readonly HostEntry[] = [],
): ConnectSection[] {
  const hostRow = (entry: HostEntry): ConnectRow => ({ type: "host", entry });
  const profileRow = (entry: ConnectEntry): ConnectRow => ({ type: "profile", entry });
  const isPinned = (group: ConnectGroup) => group.kind === "pinned";
  const sections: ConnectSection[] = [
    {
      key: "pinned",
      label: "Pinned",
      rows: [
        ...hosts.filter((entry) => entry.pinned).map(hostRow),
        ...groups.filter(isPinned).flatMap((group) => group.entries.map(profileRow)),
      ],
    },
    {
      key: "rex",
      label: REX_LABEL,
      rows: hosts.filter((entry) => !entry.pinned).map(hostRow),
    },
    ...groups
      .filter((group) => !isPinned(group))
      .map((group) => ({
        key: group.kind,
        label: group.label,
        rows: group.entries.map(profileRow),
      })),
    {
      key: "detached",
      label: DETACHED_LABEL,
      rows: detached.map((entry): ConnectRow => ({ type: "session", entry })),
    },
    {
      key: "actions",
      label: "Actions",
      rows: CONNECT_ACTIONS.map((action): ConnectRow => ({ type: "action", action })),
    },
  ];
  return sections.filter((section) => section.rows.length > 0);
}

/** The rows the keyboard moves through, in the order they stand on the page. */
export function connectRows(
  groups: readonly ConnectGroup[],
  detached: readonly DetachedEntry[] = [],
  hosts: readonly HostEntry[] = [],
): ConnectRow[] {
  return connectSections(groups, detached, hosts).flatMap((section) => section.rows);
}

/**
 * The filter after `filter` when the palette's filters are stepped through, or before
 * it, among `filters` (all of them, if not given).
 */
export function stepFilter(
  filter: ConnectFilter,
  by: 1 | -1,
  filters: readonly { readonly value: ConnectFilter }[] = CONNECT_FILTERS,
): ConnectFilter {
  const at = filters.findIndex((candidate) => candidate.value === filter);
  const next = (at + by + filters.length) % filters.length;
  return filters[next]?.value ?? "all";
}

/** What attaching to a detached session asks the core for: a new tab, or a split. */
export function reattachCommand(
  session: DetachedSession,
  how: "tab" | "split",
): {
  readonly id: string;
  readonly args: { readonly session: string; readonly backend: string; readonly place: string };
} {
  return {
    id: "lume.session.reattach",
    args: { session: session.session, backend: session.backend, place: how },
  };
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
