/**
 * What the sessions of the window's panes have reported, kept by pane.
 *
 * A session reports what its shell runs, where it is, which program has the terminal,
 * how a program's work stands and how its connection ended. Each report is folded into
 * what the pane's session has said so far, by the same rules the core folds them by: a
 * report of the whole replaces what was held and is not news; a status replaces the
 * record with its id, and a `clear` takes a record and every record beneath it.
 *
 * A pane's reports can come many times a second. Whoever shows one is told of each
 * change to that pane and reads what it needs, so reports never pass through the
 * component tree.
 *
 * Everything here is what a program printed. Its strings are made safe to show as they
 * arrive, and nothing is decided on the strength of them.
 */

import type { PaneId } from "../layout/types.ts";
import { STATUS_RECORDS_MAX } from "../limits.ts";
import { plain } from "../text/plain.ts";
import type {
  CommandState,
  Disconnected,
  ExitStatus,
  Foreground,
  Launcher,
  ProgramStatus,
  SessionEvent,
  SessionState,
} from "../transport/types.ts";
import { exitActivity, nextActivity, type Activity } from "./activity.ts";

/** What a pane's session has reported, and what the page knows of it besides. */
export interface PaneReport {
  /** The session the reports are of. */
  readonly session: string | null;
  /** What the session runs, where the application started it and so knows. */
  readonly launcher?: Launcher;
  /** The profile it was started from. */
  readonly profile?: string;
  readonly title?: string;
  /** The working directory as last reported, as a URL. */
  readonly cwd?: string;
  readonly command?: CommandState;
  /** The status records there are, the one reported longest ago first. */
  readonly statuses: readonly ProgramStatus[];
  readonly foreground?: Foreground;
  readonly disconnected?: Disconnected;
  /** How the session's program ended, once it has. */
  readonly exit?: ExitStatus;
  /** Whether the pane has something to show that the user has not looked at. */
  readonly activity: Activity;
  /**
   * Whether another view of the same session decides the terminal's size, so that this
   * pane shows a grid of another size than its own.
   */
  readonly sizedElsewhere?: boolean;
}

/** What is known of a pane whose view has attached to a session. */
export interface Attached {
  readonly session: string;
  readonly launcher?: Launcher;
  readonly profile?: string;
}

/** A pane that has reported nothing. */
const EMPTY: PaneReport = { session: null, statuses: [], activity: "idle" };

/** A copy of something whose fields may be changed. */
type Mutable<T> = { -readonly [K in keyof T]: T[K] };

/** An event's fields without its type. */
function fields<T extends { readonly type: string }>(event: T): Omit<T, "type"> {
  const { type, ...rest } = event;
  void type;
  return rest;
}

function clean(text: string | undefined): string | undefined {
  return text === undefined ? undefined : plain(text);
}

/** A status record with nothing unsafe left in its strings. */
function cleanStatus(status: ProgramStatus): ProgramStatus {
  const { app, title, message, ...rest } = status;
  return {
    ...rest,
    ...(app === undefined ? {} : { app: plain(app) }),
    ...(title === undefined ? {} : { title: plain(title) }),
    ...(message === undefined ? {} : { message: plain(message) }),
  };
}

function cleanCommand(command: CommandState): CommandState {
  if (command.state === "finished" || command.command === undefined) return command;
  return { state: "running", command: plain(command.command) };
}

function cleanDisconnected(disconnected: Disconnected): Disconnected {
  return disconnected.detail === undefined
    ? { reason: disconnected.reason }
    : { reason: disconnected.reason, detail: plain(disconnected.detail) };
}

/** Whether a status record with `id` is the one `status` names or lies beneath it. */
function covers(status: ProgramStatus, id: string | undefined): boolean {
  if (status.id === undefined) return true;
  if (id === undefined) return false;
  if (!id.startsWith(status.id)) return false;
  const rest = id.slice(status.id.length);
  return rest === "" || rest.startsWith("/");
}

/** The records once a status report is taken into account. */
function withStatus(
  records: readonly ProgramStatus[],
  status: ProgramStatus,
): readonly ProgramStatus[] {
  if (status.state === "clear") return records.filter((record) => !covers(status, record.id));
  const kept = records.filter((record) => record.id !== status.id);
  // The record nobody has touched for longest makes room.
  const room = kept.length >= STATUS_RECORDS_MAX ? kept.slice(1) : kept;
  return [...room, cleanStatus(status)];
}

/** The part of a report a state in one piece replaces. */
function fromState(report: PaneReport, state: SessionState): PaneReport {
  const title = clean(state.title);
  const { session, launcher, profile, exit, activity, sizedElsewhere } = report;
  return {
    session,
    ...(launcher ? { launcher } : {}),
    ...(profile === undefined ? {} : { profile }),
    ...(title ? { title } : {}),
    ...(state.cwd === undefined ? {} : { cwd: state.cwd }),
    ...(state.command ? { command: cleanCommand(state.command) } : {}),
    statuses: (state.statuses ?? []).slice(-STATUS_RECORDS_MAX).map(cleanStatus),
    ...(state.foreground
      ? { foreground: { pid: state.foreground.pid, name: plain(state.foreground.name) } }
      : {}),
    ...(state.disconnected ? { disconnected: cleanDisconnected(state.disconnected) } : {}),
    ...(exit === undefined ? {} : { exit }),
    activity,
    ...(sizedElsewhere ? { sizedElsewhere } : {}),
  };
}

/**
 * A report once `event` is taken into account, by the rules the core folds reports by.
 * Reports of something that happened and left nothing behind change nothing here.
 */
export function applyEvent(report: PaneReport, event: SessionEvent): PaneReport {
  switch (event.type) {
    case "title": {
      const title = plain(event.title);
      if (title !== "") return { ...report, title };
      const untitled: Mutable<PaneReport> = { ...report };
      delete untitled.title;
      return untitled;
    }
    case "cwd":
      return { ...report, cwd: event.url };
    case "command_started":
      return {
        ...report,
        command: cleanCommand(
          event.command === undefined
            ? { state: "running" }
            : { state: "running", command: event.command },
        ),
      };
    case "command_finished":
      return {
        ...report,
        command:
          event.exit === undefined
            ? { state: "finished" }
            : { state: "finished", exit: event.exit },
      };
    case "program_status":
      return { ...report, statuses: withStatus(report.statuses, fields(event)) };
    case "foreground":
      return { ...report, foreground: { pid: event.pid, name: plain(event.name) } };
    case "disconnected":
      return { ...report, disconnected: cleanDisconnected(fields(event)) };
    case "state":
      return fromState(report, event.state);
    case "bell":
    case "activity":
      return report;
  }
}

/** The reports of the window's panes. */
export class PaneReports {
  readonly #reports = new Map<PaneId, PaneReport>();
  readonly #listeners = new Map<PaneId, Set<() => void>>();
  readonly #every = new Set<(pane: PaneId) => void>();
  /** Whether the user is looking at a pane: it has the keyboard in a window in use. */
  readonly #attended: (pane: PaneId) => boolean;

  constructor(attended: (pane: PaneId) => boolean) {
    this.#attended = attended;
  }

  /** What a pane's session has reported. A pane that has said nothing has an empty report. */
  get(pane: PaneId): PaneReport {
    return this.#reports.get(pane) ?? EMPTY;
  }

  /** Begins a pane's reports afresh: its view now shows another session, or none. */
  attached(pane: PaneId, about: Attached | null): void {
    const fresh: PaneReport = about
      ? {
          session: about.session,
          ...(about.launcher ? { launcher: about.launcher } : {}),
          ...(about.profile === undefined ? {} : { profile: about.profile }),
          statuses: [],
          activity: "idle",
        }
      : EMPTY;
    this.#set(pane, fresh);
  }

  /** Takes in a report of the session a pane shows. */
  reported(pane: PaneId, event: SessionEvent): void {
    const held = this.get(pane);
    const folded = applyEvent(held, event);
    const activity = nextActivity(held.activity, event, held, this.#attended(pane));
    this.#set(pane, activity === folded.activity ? folded : { ...folded, activity });
  }

  /** Notes that the program of a pane's session has ended. */
  exited(pane: PaneId, status: ExitStatus): void {
    const held = this.get(pane);
    const activity = exitActivity(held.activity, status, this.#attended(pane));
    this.#set(pane, { ...held, exit: status, activity });
  }

  /** Notes whether another view of a pane's session decides its terminal's size. */
  sizedElsewhere(pane: PaneId, sizedElsewhere: boolean): void {
    const held = this.get(pane);
    if ((held.sizedElsewhere ?? false) === sizedElsewhere) return;
    const next: Mutable<PaneReport> = { ...held };
    if (sizedElsewhere) next.sizedElsewhere = true;
    else delete next.sizedElsewhere;
    this.#set(pane, next);
  }

  /** The user is looking at a pane: whatever it had to show has been seen. */
  attend(pane: PaneId): void {
    const held = this.#reports.get(pane);
    if (!held || held.activity === "idle") return;
    this.#set(pane, { ...held, activity: "idle" });
  }

  /** Forgets a pane that has closed. */
  forget(pane: PaneId): void {
    if (!this.#reports.delete(pane)) return;
    this.#changed(pane);
  }

  /** Forgets every pane: the window shows another workspace's panes now. */
  clear(): void {
    const panes = [...this.#reports.keys()];
    this.#reports.clear();
    for (const pane of panes) this.#changed(pane);
  }

  /** Calls `listener` whenever the report of `pane` changes. Returns how to stop. */
  subscribe(pane: PaneId, listener: () => void): () => void {
    let listeners = this.#listeners.get(pane);
    if (!listeners) {
      listeners = new Set();
      this.#listeners.set(pane, listeners);
    }
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0 && this.#listeners.get(pane) === listeners) {
        this.#listeners.delete(pane);
      }
    };
  }

  /** Calls `listener` with the pane whose report changed, for every pane. */
  subscribeAll(listener: (pane: PaneId) => void): () => void {
    this.#every.add(listener);
    return () => this.#every.delete(listener);
  }

  #set(pane: PaneId, report: PaneReport): void {
    if (this.#reports.get(pane) === report) return;
    if (report === EMPTY) this.#reports.delete(pane);
    else this.#reports.set(pane, report);
    this.#changed(pane);
  }

  #changed(pane: PaneId): void {
    for (const listener of [...(this.#listeners.get(pane) ?? [])]) listener();
    for (const listener of [...this.#every]) listener(pane);
  }
}
