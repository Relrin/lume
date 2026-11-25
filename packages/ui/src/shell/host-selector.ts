/**
 * What the selector under the workspace's name lists: this machine with the workspaces
 * of the application's own, and each Rex server with its sessions, as far as they match
 * what was typed.
 *
 * Nothing here touches the page, so all of it is tested with plain objects.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { RexHost, RexHosts, WorkspaceList } from "../core/types.ts";
import { listed, type Listed } from "../listed.ts";
import {
  connected,
  hostLine,
  linkOf,
  nearness,
  next,
  sessionName,
  sessionOnShow,
  UNKNOWN_SERVER,
  usable,
  type RexLink,
} from "../rex/model.ts";
import { SESSIONS_LISTED_MAX } from "../rex/tuning.ts";

/** Whether the selector is to be opened, and what is to be looked at in it. */
export interface SelectorDoorState {
  /** The label of the server to look at. */
  readonly asked: string | null;
  open(label: string): void;
  /** It was opened. */
  opened(): void;
}

/** The way to open the selector from elsewhere in the page: a window has one. */
export const selectorDoor: StoreApi<SelectorDoorState> = createStore<SelectorDoorState>((set) => ({
  asked: null,
  open: (label) => set({ asked: label }),
  opened: () => set({ asked: null }),
}));

/** The key of this machine among the hosts. */
export const THIS_PC = "pc";

/** What this machine is called. */
export const THIS_PC_NAME = "This PC";

/** One host of the selector's left column. */
export interface SelectorHost {
  readonly key: string;
  readonly name: string;
  /** The quieter line: what it is, how near, how much it holds. */
  readonly line: string;
  readonly link: RexLink;
  /** The server. This machine has none. */
  readonly rex: RexHost | null;
}

/** One workspace or session of the selector's right column. */
export interface SelectorRow {
  readonly key: string;
  /** The session's id, or the workspace's name. */
  readonly id: string;
  readonly name: string;
  /** Said after the name, such as a workspace's folder. */
  readonly hint: string;
  /** Whether it is what the windows show. */
  readonly on: boolean;
  /** Whether it can be called something else from here. */
  readonly renames: boolean;
  /** What taking it away is: ending a session, or deleting a workspace. */
  readonly takes: "end" | "delete" | null;
}

/** What the selector works from. */
export interface SelectorSources {
  /** The application's own workspaces. Nothing until the core has listed them. */
  readonly workspaces: WorkspaceList | null;
  readonly servers: RexHosts | null;
  /** Whose layout the windows show, if a server's. */
  readonly backend: string | null;
}

/** The last part of a folder's path, which is what tells folders apart at a glance. */
function folderName(path: string): string {
  const parts = path.split(/[\\/]/).filter((part) => part !== "");
  return parts.at(-1) ?? path;
}

function has(text: string, query: string): boolean {
  return text.toLowerCase().includes(query.trim().toLowerCase());
}

/** Everything a host holds, whatever was typed. */
function everyRow(sources: SelectorSources, host: string): SelectorRow[] {
  const { workspaces, servers, backend } = sources;
  if (host === THIS_PC) {
    return (workspaces?.workspaces ?? []).map((workspace) => {
      // A server's session may be called what a workspace is: while a server's layout is
      // on show no workspace is.
      const on = backend === null && workspace.name === workspaces?.active;
      return {
        key: `workspace:${workspace.name}`,
        id: workspace.name,
        name: workspace.name,
        hint: workspace.root === null ? "" : folderName(workspace.root),
        on,
        // Only the workspace on show is renamed, and only another is deleted.
        renames: on,
        takes: on ? null : "delete",
      };
    });
  }
  const server = servers?.hosts.find((found) => found.id === host);
  if (!server || !connected(server)) return [];
  const shown = sessionOnShow(server, backend);
  return server.sessions.map((session) => ({
    key: `session:${session.id}`,
    id: session.id,
    name: sessionName(session),
    hint: "",
    on: session.id === shown,
    renames: true,
    takes: "end" as const,
  }));
}

/** Every host there is: this machine first, then the servers. */
function everyHost(sources: SelectorSources): SelectorHost[] {
  const count = sources.workspaces?.workspaces.length;
  return [
    {
      key: THIS_PC,
      name: THIS_PC_NAME,
      line: count === undefined ? "" : count === 1 ? "1 workspace" : `${count} workspaces`,
      link: "up",
      rex: null,
    },
    ...(sources.servers?.hosts ?? []).map((host): SelectorHost => ({
      key: host.id,
      name: host.label,
      line: hostLine(host),
      link: linkOf(host),
      rex: host,
    })),
  ];
}

/** The hosts to list: those whose name matches, or the name of something they hold. */
export function selectorHosts(sources: SelectorSources, query: string): SelectorHost[] {
  return everyHost(sources).filter(
    (host) =>
      has(host.name, query) || everyRow(sources, host.key).some((row) => has(row.name, query)),
  );
}

/**
 * The host whose workspaces or sessions are listed: the one the user is looking at, if
 * it is still listed, else the one on show, else the first that is. With nothing listed
 * it is the one the user is looking at all the same, where what was typed can be made.
 */
export function browsedHost(
  sources: SelectorSources,
  query: string,
  wanted: string | null,
): SelectorHost | undefined {
  const hosts = selectorHosts(sources, query);
  const every = everyHost(sources);
  const find = (among: readonly SelectorHost[], key: string | null) =>
    among.find((host) => host.key === key);
  const onShow = sources.backend ?? THIS_PC;
  return (
    find(hosts, wanted) ??
    find(hosts, onShow) ??
    hosts[0] ??
    find(every, wanted) ??
    find(every, onShow) ??
    every[0]
  );
}

/** What a host holds that matches. A host that matches by its own name lists it all. */
export function selectorRows(
  sources: SelectorSources,
  host: SelectorHost,
  query: string,
): SelectorRow[] {
  const rows = everyRow(sources, host.key);
  const matching = rows.filter((row) => has(row.name, query));
  return matching.length === 0 && has(host.name, query) ? rows : matching;
}

/** What would be made of what was typed: nothing if a host has something of that name. */
export function creatable(
  sources: SelectorSources,
  host: SelectorHost,
  query: string,
): string | null {
  const name = query.trim();
  if (name === "") return null;
  if (host.rex && !connected(host.rex)) return null;
  const taken = everyRow(sources, host.key).some(
    (row) => row.name.toLowerCase() === name.toLowerCase(),
  );
  return taken ? null : name;
}

/** Something that can be done in the selector's right column, besides what it lists. */
export type SelectorAct =
  | { readonly kind: "create"; readonly label: string; readonly name: string }
  | { readonly kind: "new-session"; readonly label: string }
  | { readonly kind: "connect"; readonly label: string; readonly agree: boolean }
  | { readonly kind: "disconnect"; readonly label: string }
  | { readonly kind: "edit"; readonly label: string }
  | { readonly kind: "command"; readonly label: string; readonly command: string };

/** The commands about workspaces, with what the selector calls each. */
const WORKSPACE_COMMANDS: readonly { readonly command: string; readonly label: string }[] = [
  { command: "lume.workspace.open-folder", label: "Open folder…" },
  { command: "lume.workspace.new", label: "New workspace…" },
  { command: "lume.workspace.set-root", label: "Set workspace folder…" },
];

/** The commands that are about the workspace on show, and mean nothing without one. */
const OF_THE_SHOWN: readonly string[] = ["lume.workspace.set-root"];

/**
 * What can be done with a host. `known` is which commands there are.
 */
export function selectorActs(
  sources: SelectorSources,
  host: SelectorHost,
  query: string,
  known: (command: string) => boolean,
): SelectorAct[] {
  const acts: SelectorAct[] = [];
  const name = creatable(sources, host, query);
  const place = host.name;
  if (name !== null) acts.push({ kind: "create", label: `Create “${name}” on ${place}`, name });
  const server = host.rex;
  if (server === null) {
    for (const { command, label } of WORKSPACE_COMMANDS) {
      if (!known(command)) continue;
      if (sources.backend !== null && OF_THE_SHOWN.includes(command)) continue;
      acts.push({ kind: "command", label, command });
    }
    return acts;
  }
  if (!usable(server)) {
    if (server.written) acts.push({ kind: "edit", label: "Edit connection…" });
    return acts;
  }
  if (connected(server)) acts.push({ kind: "new-session", label: `New session on ${place}` });
  const step = next(server);
  if (step) acts.push({ kind: "connect", label: step.label, agree: step.kind === "agree" });
  const kind = server.state.kind;
  if (kind === "connected" || kind === "waiting") {
    acts.push({ kind: "disconnect", label: "Disconnect" });
  } else if (kind === "connecting") {
    acts.push({ kind: "disconnect", label: "Cancel" });
  }
  if (server.written && linkOf(server) === "down") {
    acts.push({ kind: "edit", label: "Edit connection…" });
  }
  return acts;
}

/** What is listed of a host's rows, the one on show among it. */
export function listedRows(rows: readonly SelectorRow[]): Listed<SelectorRow> {
  return listed(rows, SESSIONS_LISTED_MAX, (row) => row.on);
}

/** Which of the rows listed is marked first: the one on show, else the first. */
export function firstRow(rows: readonly SelectorRow[]): number {
  return Math.max(
    0,
    rows.findIndex((row) => row.on),
  );
}

/** What stands in the button under which the selector opens. */
export interface SelectorLabel {
  readonly host: string;
  readonly name: string;
  /** How near a host elsewhere is, or that it is not there. */
  readonly meta: string;
  readonly link: RexLink;
  /** What the dot says, in words. */
  readonly said: string;
}

/**
 * The button's words. `server` is the server whose layout is on show, if one is and is
 * known; `hostConnected` is whether the session host of this machine is reached.
 */
export function selectorLabel(
  name: string,
  backend: string | null,
  server: RexHost | undefined,
  hostConnected: boolean,
): SelectorLabel {
  if (backend === null) {
    return {
      host: THIS_PC_NAME,
      name,
      meta: "",
      link: hostConnected ? "up" : "down",
      said: hostConnected ? "Session host connected" : "Session host disconnected",
    };
  }
  if (!server) return { host: UNKNOWN_SERVER, name, meta: "", link: "down", said: "Not connected" };
  const link = linkOf(server);
  return {
    host: server.label,
    name,
    meta: link === "busy" ? "" : link === "up" ? nearness(server) : "offline",
    link,
    said: link === "busy" ? "Reconnecting" : link === "up" ? "Connected" : "Not connected",
  };
}
