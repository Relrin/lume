/**
 * What is said of a Rex server: how it stands, in a word and in a sentence, and what
 * can be done about it.
 *
 * Nothing here touches the page, so all of it is tested with plain objects.
 */

import type { RexHost, RexHosts, RexSession } from "../core/types.ts";

/** Whether a server can be used: it can, it is on its way, or it cannot. */
export type RexLink = "up" | "busy" | "down";

/** What choosing a server that is not connected does. */
export type RexNext =
  /** It can be connected, or tried again. */
  | { readonly kind: "connect"; readonly label: string }
  /** It can be connected only without encryption, which the user has to agree to. */
  | { readonly kind: "agree"; readonly label: string };

/** What an id begins with that stands for an address nothing can be reached at. */
const UNUSABLE = "rex:?";

/** Whether a server's address is one that can be reached at all. */
export function usable(host: Pick<RexHost, "id">): boolean {
  return !host.id.startsWith(UNUSABLE);
}

/** Whether a server answers now, so that its sessions can be opened. */
export function connected(host: Pick<RexHost, "state">): boolean {
  return host.state.kind === "connected";
}

/** Whether a server can be used, for the dot beside its name. */
export function linkOf(host: Pick<RexHost, "state">): RexLink {
  switch (host.state.kind) {
    case "connected":
      return "up";
    case "connecting":
    case "waiting":
      return "busy";
    default:
      return "down";
  }
}

/** "1 session", "3 sessions". */
export function sessionCount(host: Pick<RexHost, "sessions">): string {
  return host.sessions.length === 1 ? "1 session" : `${host.sessions.length} sessions`;
}

/** Why a server is not connected, in a word or two, by the core's code for it. */
const FAILURES: Readonly<Record<string, string>> = {
  unreachable: "Unreachable",
  confirmation_required: "Not encrypted",
  untrusted: "Not trusted",
  forbidden: "Refused",
  rejected: "Refused",
  version_mismatch: "Other version",
  ssh: "SSH failed",
  unsupported: "Not supported",
  not_installed: "Not installed",
  bad_request: "Address not usable",
};

/** How a server stands, in a word or two. */
export function standing(host: Pick<RexHost, "state" | "sessions">): string {
  switch (host.state.kind) {
    case "connected":
      return sessionCount(host);
    case "waiting":
      // That it is being connected to again is shown, not said.
      return sessionCount(host);
    case "connecting":
      return "Connecting…";
    case "idle":
      return "Not connected";
    case "absent":
      return "Not running";
    case "failed":
      return FAILURES[host.state.code] ?? "Failed";
  }
}

/** An address of this machine, the way one is written: by name or by number. */
const LOOPBACK =
  /^(?:[a-z]+:\/\/)?(?:[^@/]*@)?(?:localhost|[\w.-]+\.localhost|127(?:\.\d{1,3}){3}|\[::1\])(?::\d+)?(?:\/|$)/i;

/**
 * Whether a server is on this machine, or in a Linux distribution on it, which is
 * reached at an address of this machine: how near it is says nothing.
 */
export function onThisMachine(host: Pick<RexHost, "local" | "address">): boolean {
  return host.local || /^(?:npipe|unix):/i.test(host.address) || LOOPBACK.test(host.address);
}

/**
 * How near a server elsewhere is, or that a server is not there. Nothing while that is
 * not known.
 */
export function nearness(
  host: Pick<RexHost, "state" | "local" | "address" | "latency_ms">,
): string {
  switch (host.state.kind) {
    case "connected":
      if (host.latency_ms === null || onThisMachine(host)) return "";
      return host.latency_ms === 0 ? "<1 ms" : `${host.latency_ms} ms`;
    case "absent":
      return "offline";
    case "failed":
      // One that waits for the user's word may well be there.
      return host.state.code === "confirmation_required" ? "" : "offline";
    default:
      return "";
  }
}

/** A server in a line: "Rex · 12 ms · 3 sessions", or why not. */
export function hostLine(host: RexHost): string {
  if (!connected(host)) return `Rex · ${standing(host)}`;
  return ["Rex", nearness(host), sessionCount(host)].filter((part) => part !== "").join(" · ");
}

/** A message as a sentence that has an end. */
function ended(message: string): string {
  const text = message.trim();
  const whole = text.charAt(0).toUpperCase() + text.slice(1);
  return /[.!?…]$/.test(whole) ? whole : `${whole}.`;
}

/** What a connection without encryption means, as it is put to the user. */
export const UNENCRYPTED =
  "The connection would not be encrypted. Whoever is on the network between could read " +
  "what its terminals show and what you type, and change it.";

/** Why a server's sessions cannot be opened now, in a sentence. None while they can. */
export function reason(host: Pick<RexHost, "id" | "state">): string | null {
  const state = host.state;
  switch (state.kind) {
    case "connected":
      return null;
    case "connecting":
      return "Connecting…";
    case "idle":
      return "Not connected. Its sessions are listed once it is.";
    case "absent":
      return "No Rex server runs on this computer. It is listed as soon as one does.";
    case "waiting":
      return `The connection was lost and is being made again. ${ended(state.error)}`;
    case "failed":
      break;
  }
  if (!usable(host)) return `The address cannot be used. ${ended(state.error)}`;
  switch (state.code) {
    case "confirmation_required":
      return UNENCRYPTED;
    case "unreachable":
      return `Unreachable. ${ended(state.error)}`;
    case "untrusted":
      return `The server's certificate is not trusted. ${ended(state.error)}`;
    case "forbidden":
    case "rejected":
      return `The server does not let this computer in. ${ended(state.error)}`;
    case "version_mismatch":
      return `The server speaks another version of Rex. ${ended(state.error)}`;
    case "not_installed":
      return `${ended(state.error)} Install it there, and try again.`;
    case "ssh":
      return `${ended(state.error)} Connecting once in a terminal shows what SSH asks for.`;
    default:
      return ended(state.error);
  }
}

/** What choosing a server does that is not connected. None where there is nothing to do. */
export function next(host: Pick<RexHost, "id" | "state">): RexNext | null {
  if (!usable(host)) return null;
  switch (host.state.kind) {
    case "idle":
      return { kind: "connect", label: "Connect" };
    case "absent":
      return { kind: "connect", label: "Retry" };
    case "failed":
      return host.state.code === "confirmation_required"
        ? { kind: "agree", label: "Connect without encryption" }
        : { kind: "connect", label: "Retry" };
    default:
      return null;
  }
}

/** Whether there is a connection to end, or one being made to give up. */
export function canDisconnect(host: Pick<RexHost, "state">): boolean {
  const kind = host.state.kind;
  return kind === "connected" || kind === "connecting" || kind === "waiting";
}

const SYSTEMS: Readonly<Record<string, string>> = {
  windows: "Windows",
  linux: "Linux",
  darwin: "macOS",
  macos: "macOS",
  freebsd: "FreeBSD",
};

/** The system a server runs on, as it is called: "Windows" for "windows_amd64". */
export function systemName(platform: string): string {
  const name = platform.split("_")[0] ?? "";
  return SYSTEMS[name.toLowerCase()] ?? (name === "" ? "—" : name);
}

/** The server whose layout is on show, if one is. */
export function hostOnShow(list: RexHosts | null, backend: string | null): RexHost | undefined {
  return backend === null ? undefined : list?.hosts.find((host) => host.id === backend);
}

/**
 * The session of a server that is on show. `backend` is whose layout the windows show,
 * which is known before the server's own word of what is shown.
 */
export function sessionOnShow(host: RexHost, backend: string | null): string | null {
  return host.id === backend ? host.on_show : null;
}

/** Which of a server's sessions is marked first: the one on show, else the first. */
export function firstMarked(host: RexHost, backend: string | null): number {
  const shown = sessionOnShow(host, backend);
  return Math.max(
    0,
    host.sessions.findIndex((session) => session.id === shown),
  );
}

/** What a session is called, which its server may have left empty. */
export function sessionName(session: Pick<RexSession, "label">): string {
  return session.label.trim() === "" ? "unnamed" : session.label;
}
