/**
 * What a pane is called, where it is and how it stands, as its header and its tab show
 * it.
 *
 * This is the one place that decides. It reads what the pane's session has reported,
 * what the layout says of the pane, and the profile the pane was started from, and says
 * what to show. It decides nothing else: a report is what a program printed.
 */

import type { Machine, Profile, ProfileList } from "../core/types.ts";
import { findProfile } from "../profiles/store.ts";
import type { DisconnectReason, ExitStatus, Launcher, ProgramStatus } from "../transport/types.ts";
import type { Activity } from "./activity.ts";
import {
  distributionIcon,
  isShell,
  plainIcon,
  programIcon,
  programKey,
  type PaneIcon,
  type ProgramIcon,
} from "./programs.ts";
import type { PaneReport } from "./store.ts";
import { homeShortened, lastFolder, localPlace, parseFileUrl } from "./where.ts";

/** What a pane is called when nothing says what runs in it. */
export const UNTITLED_PANE = "Terminal";

/** Where a pane's session runs: here, in a Linux distribution here, or on another machine. */
export type PaneKind = Launcher["type"];

/** What the layout says of a pane. */
export interface PaneFacts {
  /** The name the user gave the pane, if they did. */
  readonly name: string | null;
  /** The directory the pane was started in. */
  readonly startCwd: string | null;
  /** What the pane says it runs, where it names it. */
  readonly launcher: Launcher | null;
  /** The profile the pane says it runs, where it names one. */
  readonly profile: string | null;
}

/** What describing a pane reads besides the pane. */
export interface DescribeContext {
  readonly machine: Machine;
  /** Whether this machine writes its paths as Windows does. */
  readonly windows: boolean;
  readonly profiles: ProfileList | null;
  /** The user's own pictures for programs, by name. */
  readonly icons: ReadonlyMap<string, ProgramIcon>;
}

/** The status a program reported that a header shows. */
export interface StatusMark {
  readonly state: "blocked" | "error" | "working" | "done";
  readonly label: string;
  /** What the program said about it, where it said something. */
  readonly tooltip: string | null;
}

/** How a pane's program ended, and what may be done about it. */
export interface PaneEnd {
  readonly reason: string;
  /** The last words of the client that made the connection, where there were some. */
  readonly detail: string | null;
  /** Whether starting it again connects again, or starts a program again. */
  readonly again: "reconnect" | "restart";
}

/** A pane as its header and tab show it. */
export interface PaneDescription {
  readonly kind: PaneKind;
  /** What the pane is called. */
  readonly name: string;
  /** The tile for what runs in it. */
  readonly icon: PaneIcon;
  /** Where it is, shortened, or nothing when nothing says. */
  readonly where: string;
  /** Where it is in full. */
  readonly whereInFull: string;
  /** Whether the shell is running a command line. */
  readonly running: boolean;
  /** The exit code of the command line that ended last, when it failed. */
  readonly failed: number | null;
  readonly status: StatusMark | null;
  /** How the pane's program ended, once it has. */
  readonly ended: PaneEnd | null;
  readonly activity: Activity;
}

/** What each status is called, and how much it presses: the first that holds is shown. */
const STATUS_ORDER: readonly { readonly state: StatusMark["state"]; readonly label: string }[] = [
  { state: "blocked", label: "Waiting on you" },
  { state: "error", label: "Failed" },
  { state: "working", label: "Working" },
  { state: "done", label: "Finished" },
];

/** What each way a connection ends is called. */
const DISCONNECTS: Readonly<Record<Exclude<DisconnectReason, "exited">, string>> = {
  closed: "Connection closed",
  auth_failed: "Authentication failed",
  host_key: "Host key changed",
  unreachable: "Host unreachable",
  connection_lost: "Connection lost",
  failed: "Could not connect",
};

/** A program's name as it is shown: no folder, no `.exe`. */
export function programName(program: string): string {
  const base = program.split(/[\\/]/).at(-1) ?? program;
  return base.replace(/\.exe$/i, "");
}

/** The program a command line starts: its first word, without a folder. */
function commandProgram(command: string | undefined): string | undefined {
  const first = command?.trim().split(/\s+/)[0];
  return first ? programName(first) : undefined;
}

function launcherProgram(launcher: Launcher | undefined): string | undefined {
  if (launcher?.type !== "shell" || launcher.program.trim() === "") return undefined;
  return programName(launcher.program);
}

/** The status record that presses most, if a program has said anything. */
function statusMark(statuses: readonly ProgramStatus[]): StatusMark | null {
  for (const { state, label } of STATUS_ORDER) {
    // The newest record of a state is the one whose words are shown.
    const record = [...statuses].reverse().find((status) => status.state === state);
    if (record)
      return { state, label, tooltip: record.message ?? record.title ?? record.app ?? null };
  }
  return null;
}

/** How an exit status is said. */
function endedWith(status: ExitStatus | undefined): string {
  if (status === undefined || status === "unknown") return "Ended";
  if ("signal" in status) return `Ended by signal ${status.signal}`;
  return status.code === 0 ? "Ended" : `Ended with code ${status.code}`;
}

function ending(report: PaneReport, kind: PaneKind): PaneEnd | null {
  if (report.exit === undefined) return null;
  const disconnected = report.disconnected;
  const reason =
    disconnected && disconnected.reason !== "exited"
      ? DISCONNECTS[disconnected.reason]
      : endedWith(report.exit);
  return {
    reason,
    detail: disconnected?.detail ?? null,
    again: kind === "ssh" ? "reconnect" : "restart",
  };
}

/** Where a shell on this machine says it is, or where its pane was started. */
function localWhere(
  report: PaneReport,
  facts: PaneFacts,
  context: DescribeContext,
): { short: string; full: string } {
  const { machine, windows } = context;
  const place = report.cwd === undefined ? undefined : localPlace(report.cwd, machine, windows);
  if (place?.elsewhere) {
    const full = `${place.elsewhere}:${place.path}`;
    return { short: full, full };
  }
  const path = place?.path ?? facts.startCwd;
  if (!path) return { short: "", full: "" };
  return { short: homeShortened(path, machine.home, windows), full: path };
}

/**
 * Whether a title is only the path of a program here: what the console calls a window
 * whose program has given it no title of its own.
 */
function isProgramPath(title: string): boolean {
  return /^([A-Za-z]:[\\/]|\/).*\.exe$/i.test(title.trim());
}

/** Where a shell on another machine, or in a distribution, says it is. */
function remoteWhere(
  report: PaneReport,
  facts: PaneFacts,
  launcher: Extract<Launcher, { type: "wsl" | "ssh" }>,
  place: string,
  named: boolean,
): { short: string; full: string } {
  const parsed = report.cwd === undefined ? undefined : parseFileUrl(report.cwd);
  // A distribution's pane starts where it was asked to; another machine's, wherever
  // that machine puts it.
  const path = parsed?.path ?? (launcher.type === "wsl" ? facts.startCwd : null);
  if (!path) {
    const title = report.title ?? "";
    if (title !== "" && !isProgramPath(title)) return { short: title, full: title };
    // Nothing says where: another machine is at least called by its name.
    const host =
      launcher.type === "ssh" && !named
        ? `${launcher.user === undefined ? "" : `${launcher.user}@`}${launcher.host}`
        : "";
    return { short: host, full: host };
  }
  const full = `${place}:${path}`;
  return { short: named ? path : full, full };
}

/** What the program a pane runs last said it is, where it said. */
function appOf(statuses: readonly ProgramStatus[]): string | undefined {
  return statuses.findLast((status) => status.app !== undefined && status.app !== "")?.app;
}

/**
 * The tile for what runs in a pane. A program other than a shell is known by what it
 * says it is or by its name, and given a plain tile when neither is known; a shell at
 * its prompt shows where it runs: here, in a distribution, or on another machine.
 */
function paneIcon(
  program: string | undefined,
  launcher: Launcher | undefined,
  statuses: readonly ProgramStatus[],
  context: DescribeContext,
): PaneIcon {
  if (program !== undefined) {
    const shell = isShell(program);
    const known = programIcon(program, context.icons, shell ? undefined : appOf(statuses));
    if (known) return known;
    if (!shell) return plainIcon(program);
  }
  if (launcher?.type === "wsl") return distributionIcon(launcher.distro) ?? "terminal";
  return launcher?.type === "ssh" ? "remote" : "terminal";
}

/** Describes a pane. */
export function describePane(
  report: PaneReport,
  facts: PaneFacts,
  context: DescribeContext,
): PaneDescription {
  const profile: Profile | undefined = findProfile(
    context.profiles,
    report.profile ?? facts.profile,
  );
  const launcher = report.launcher ?? facts.launcher ?? profile?.launcher;
  const kind: PaneKind = launcher?.type ?? "shell";
  const command = report.command;
  const running = command?.state === "running" && report.exit === undefined;

  let program: string | undefined;
  let name: string;
  let where: { short: string; full: string };
  if (launcher?.type === "wsl" || launcher?.type === "ssh") {
    const place = launcher.type === "wsl" ? launcher.distro : launcher.host;
    program = running ? commandProgram(command.command) : undefined;
    name = program ?? profile?.name ?? place;
    where = remoteWhere(report, facts, launcher, place, name === place);
  } else {
    const foreground = report.foreground ? programName(report.foreground.name) : undefined;
    const shell = launcherProgram(launcher);
    // A program a shell runs is not always found below it: under Git for Windows it
    // loses its parent as it starts. While the shell says it runs a command line and
    // nothing but a shell is seen, the command line says what runs.
    const onlyShellSeen =
      foreground === undefined ||
      (shell === undefined ? isShell(foreground) : programKey(foreground) === programKey(shell));
    const commanded = running && onlyShellSeen ? commandProgram(command.command) : undefined;
    program = commanded ?? foreground ?? shell;
    name = program ?? profile?.name ?? UNTITLED_PANE;
    where = localWhere(report, facts, context);
  }

  const icon = paneIcon(program, launcher, report.statuses, context);
  const failed =
    !running && command?.state === "finished" && command.exit !== undefined && command.exit !== 0
      ? command.exit
      : null;
  const ended = ending(report, kind);
  return {
    kind,
    name: facts.name ?? name,
    icon,
    where: where.short,
    whereInFull: where.full,
    running,
    failed: ended ? null : failed,
    status: ended ? null : statusMark(report.statuses),
    ended,
    activity: report.activity,
  };
}

/** What a tab, or the window, is called after a pane: its name, and its last folder. */
export function paneLabel(description: PaneDescription): string {
  if (description.where === "") return description.name;
  const folder = lastFolder(description.where);
  return folder === description.name ? folder : `${description.name} · ${folder}`;
}

/** Whether two programs are the same one, as pictures and names compare them. */
export function sameProgram(a: string, b: string): boolean {
  return programKey(a) === programKey(b);
}
