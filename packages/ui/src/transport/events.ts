/**
 * Reads what a session reports besides its output: its title, its directory, the
 * commands its shell runs, a program's status, the program that has the terminal, and
 * how the session ended.
 *
 * A report is checked field by field, as the core checks what the page sends. One that
 * is not what it must be is refused whole, and a field the report does not have is
 * left out of what is returned, so that a report reads as it was sent.
 */

import type {
  BlockKind,
  CommandState,
  Disconnected,
  DisconnectReason,
  Foreground,
  ProgramState,
  ProgramStatus,
  SessionEvent,
  SessionState,
} from "./types.ts";

const PROGRAM_STATES: readonly ProgramState[] = [
  "idle",
  "working",
  "done",
  "blocked",
  "error",
  "clear",
];
const BLOCK_KINDS: readonly BlockKind[] = ["permission", "question", "auth"];
const DISCONNECT_REASONS: readonly DisconnectReason[] = [
  "exited",
  "closed",
  "auth_failed",
  "host_key",
  "unreachable",
  "connection_lost",
  "failed",
];

/** Why a report was refused. */
export class EventError extends Error {}

type Fields = Record<string, unknown>;

function fields(value: unknown, what: string): Fields {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new EventError(`${what} is not an object`);
  }
  return value as Fields;
}

/** Refuses an object that has a field other than the ones named. */
function only(object: Fields, what: string, names: readonly string[]): void {
  for (const name of Object.keys(object)) {
    if (!names.includes(name)) throw new EventError(`${what} has no field called ${name}`);
  }
}

function text(object: Fields, name: string, what: string): string {
  const value = object[name];
  if (typeof value !== "string") throw new EventError(`${what} needs ${name} as text`);
  return value;
}

function optionalText(object: Fields, name: string, what: string): string | undefined {
  return object[name] === undefined ? undefined : text(object, name, what);
}

function integer(object: Fields, name: string, what: string): number {
  const value = object[name];
  if (typeof value !== "number" || !Number.isInteger(value)) {
    throw new EventError(`${what} needs ${name} as a whole number`);
  }
  return value;
}

function optionalInteger(object: Fields, name: string, what: string): number | undefined {
  return object[name] === undefined ? undefined : integer(object, name, what);
}

function choice<T extends string>(
  object: Fields,
  name: string,
  what: string,
  choices: readonly T[],
): T {
  const value = object[name];
  const found = choices.find((candidate) => candidate === value);
  if (found === undefined) throw new EventError(`${what} has a ${name} nobody knows`);
  return found;
}

/** An object without the fields that have no value. */
function present<T extends object>(object: { [K in keyof T]: T[K] | undefined }): T {
  const out: Fields = {};
  for (const [name, value] of Object.entries(object)) {
    if (value !== undefined) out[name] = value;
  }
  return out as T;
}

const STATUS_FIELDS = ["state", "id", "kind", "progress", "app", "title", "message"];

function programStatus(object: Fields, what: string): ProgramStatus {
  const progress = optionalInteger(object, "progress", what);
  if (progress !== undefined && (progress < 0 || progress > 100)) {
    throw new EventError(`${what} has a progress outside 0 to 100`);
  }
  return present<ProgramStatus>({
    state: choice(object, "state", what, PROGRAM_STATES),
    id: optionalText(object, "id", what),
    kind: object.kind === undefined ? undefined : choice(object, "kind", what, BLOCK_KINDS),
    progress,
    app: optionalText(object, "app", what),
    title: optionalText(object, "title", what),
    message: optionalText(object, "message", what),
  });
}

function foreground(object: Fields, what: string): Foreground {
  const pid = integer(object, "pid", what);
  if (pid < 0) throw new EventError(`${what} has a process id below zero`);
  return { pid, name: text(object, "name", what) };
}

function disconnected(object: Fields, what: string): Disconnected {
  return present<Disconnected>({
    reason: choice(object, "reason", what, DISCONNECT_REASONS),
    detail: optionalText(object, "detail", what),
  });
}

function commandState(value: unknown): CommandState {
  const what = "a command's state";
  const object = fields(value, what);
  const state = choice(object, "state", what, ["running", "finished"] as const);
  if (state === "running") {
    only(object, what, ["state", "command"]);
    return present<CommandState>({ state, command: optionalText(object, "command", what) });
  }
  only(object, what, ["state", "exit"]);
  return present<CommandState>({ state, exit: optionalInteger(object, "exit", what) });
}

function sessionState(value: unknown): SessionState {
  const what = "a session's state";
  const object = fields(value, what);
  only(object, what, ["title", "cwd", "command", "statuses", "foreground", "disconnected"]);
  const { statuses } = object;
  if (statuses !== undefined && !Array.isArray(statuses)) {
    throw new EventError(`${what} needs statuses as a list`);
  }
  const part = <T>(name: string, read: (object: Fields, what: string) => T): T | undefined => {
    if (object[name] === undefined) return undefined;
    const inner = fields(object[name], `${what}'s ${name}`);
    return read(inner, `${what}'s ${name}`);
  };
  return present<SessionState>({
    title: optionalText(object, "title", what),
    cwd: optionalText(object, "cwd", what),
    command: object.command === undefined ? undefined : commandState(object.command),
    statuses: statuses?.map((status: unknown) => {
      const record = fields(status, "a status record");
      only(record, "a status record", STATUS_FIELDS);
      return programStatus(record, "a status record");
    }),
    foreground: part("foreground", (inner, name) => {
      only(inner, name, ["pid", "name"]);
      return foreground(inner, name);
    }),
    disconnected: part("disconnected", (inner, name) => {
      only(inner, name, ["reason", "detail"]);
      return disconnected(inner, name);
    }),
  });
}

/** Reads one report from the JSON it travels as. Throws an {@link EventError}. */
export function sessionEvent(value: unknown): SessionEvent {
  const object = fields(value, "a session event");
  const { type } = object;
  const what = `a ${String(type)} event`;
  const exactly = (...names: string[]): void => only(object, what, ["type", ...names]);
  switch (type) {
    case "title":
      exactly("title");
      return { type, title: text(object, "title", what) };
    case "cwd":
      exactly("url");
      return { type, url: text(object, "url", what) };
    case "bell":
    case "activity":
      // An event that carries nothing is known by its name alone, as it is to the core.
      return { type };
    case "command_started":
      exactly("command");
      return present({ type, command: optionalText(object, "command", what) });
    case "command_finished":
      exactly("exit");
      return present({ type, exit: optionalInteger(object, "exit", what) });
    case "program_status":
      exactly(...STATUS_FIELDS);
      return { type, ...programStatus(object, what) };
    case "foreground":
      exactly("pid", "name");
      return { type, ...foreground(object, what) };
    case "disconnected":
      exactly("reason", "detail");
      return { type, ...disconnected(object, what) };
    case "state":
      exactly("state");
      return { type, state: sessionState(object.state) };
    default:
      throw new EventError("not a session event");
  }
}
