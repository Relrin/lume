/**
 * The values that cross between this page and the core: sizes, what a view tells its
 * session about itself, and what a session reports back.
 *
 * The core speaks JSON with snake_case keys and colours as `#rrggbb`. The functions here
 * turn those shapes into the ones the rest of the page uses and back, so nothing outside
 * the transport sees the wire spelling.
 */

/** A terminal's size in cells. Each side is between 1 and {@link MAX_DIMENSION}. */
export interface TermSize {
  readonly cols: number;
  readonly rows: number;
}

/** The most columns or rows a terminal may have. A rule of the wire format. */
export const MAX_DIMENSION = 4096;

/** One cell's size in device pixels. Zero means it has not been measured. */
export interface CellPx {
  readonly w: number;
  readonly h: number;
}

export interface Rgb {
  readonly r: number;
  readonly g: number;
  readonly b: number;
}

/** The colours a session may be asked about. `ansi` has exactly sixteen entries. */
export interface Palette {
  readonly fg: Rgb;
  readonly bg: Rgb;
  readonly cursor: Rgb;
  readonly ansi: readonly Rgb[];
}

export type ColorScheme = "light" | "dark";

/** What a view tells its session about itself. */
export interface ClientFacts {
  readonly size: TermSize;
  readonly cellPx: CellPx;
  readonly palette: Palette;
  readonly scheme: ColorScheme;
  readonly focused: boolean;
  /** Whether the view is on screen, as opposed to in a hidden or minimised window. */
  readonly visible: boolean;
}

/** How a session's program ended. */
export type ExitStatus = { readonly code: number } | { readonly signal: number } | "unknown";

/** How a program's work stands, as the program reports it. `clear` takes a report back. */
export type ProgramState = "idle" | "working" | "done" | "blocked" | "error" | "clear";

/** What a blocked program is waiting for. */
export type BlockKind = "permission" | "question" | "auth";

/** One report of a program's status. A report replaces the record with its `id` whole. */
export interface ProgramStatus {
  readonly state: ProgramState;
  /** Which record this is, as a path such as `build/test`. Without one, the root record. */
  readonly id?: string;
  readonly kind?: BlockKind;
  /** How far the work has come, from 0 to 100. */
  readonly progress?: number;
  /** A short name for the program, such as `cargo`. */
  readonly app?: string;
  readonly title?: string;
  /** One line for a person to read. */
  readonly message?: string;
}

/** The program that has a terminal: the one that reads what is typed. */
export interface Foreground {
  readonly pid: number;
  /** The program's name, without its folder. */
  readonly name: string;
}

/** Why the connection a session ran over ended. */
export type DisconnectReason =
  "exited" | "closed" | "auth_failed" | "host_key" | "unreachable" | "connection_lost" | "failed";

export interface Disconnected {
  readonly reason: DisconnectReason;
  /** The last line the client printed, where it had one. */
  readonly detail?: string;
}

/** Where a shell is with the command lines it runs. */
export type CommandState =
  | { readonly state: "running"; readonly command?: string }
  | { readonly state: "finished"; readonly exit?: number };

/** What a session's reports add up to: the latest of each kind. */
export interface SessionState {
  readonly title?: string;
  /** The working directory as last reported, as a URL. */
  readonly cwd?: string;
  readonly command?: CommandState;
  /** The status records there are, the one reported longest ago first. */
  readonly statuses?: readonly ProgramStatus[];
  readonly foreground?: Foreground;
  readonly disconnected?: Disconnected;
}

/**
 * Something a session reports besides its output. All of it is what a program printed
 * or what the system said about it: good for showing, and for nothing else.
 */
export type SessionEvent =
  | { readonly type: "title"; readonly title: string }
  | { readonly type: "cwd"; readonly url: string }
  | { readonly type: "bell" }
  | { readonly type: "command_started"; readonly command?: string }
  | { readonly type: "command_finished"; readonly exit?: number }
  | ({ readonly type: "program_status" } & ProgramStatus)
  | ({ readonly type: "foreground" } & Foreground)
  /** The session printed something while its view did not have the keyboard. */
  | { readonly type: "activity" }
  | ({ readonly type: "disconnected" } & Disconnected)
  /**
   * Everything the session has reported so far, in one piece. It replaces what was
   * held and is not news: nothing in it has just happened.
   */
  | { readonly type: "state"; readonly state: SessionState };

/** Which side answers the questions a program asks its terminal. */
export type Responder = "client" | "backend";

/** What a backend can do. Code branches on these, never on which backend it has. */
export interface BackendCaps {
  readonly persistent: boolean;
  readonly multiClient: boolean;
  readonly serverLayout: boolean;
  readonly responder: Responder;
  readonly hydrate: { readonly snapshot: boolean; readonly vtReplay: boolean };
}

/** A session as a backend lists it. */
export interface SessionInfo {
  readonly backend: string;
  /** Sixteen hexadecimal digits. */
  readonly id: string;
  readonly program: string;
  readonly pid: number | null;
  readonly size: TermSize;
  readonly createdUnixMs: number;
  readonly attached: boolean;
  /** Set once the program has ended. */
  readonly exit: ExitStatus | null;
}

/**
 * What a session runs: a program on this machine, the shell of a Linux distribution
 * on it, or a shell on another machine. Left out, the core starts the default profile.
 */
export type Launcher =
  | {
      readonly type: "shell";
      readonly program: string;
      readonly args: readonly string[];
      readonly login: boolean;
    }
  | { readonly type: "wsl"; readonly distro: string; readonly user?: string }
  | {
      readonly type: "ssh";
      readonly host: string;
      readonly user?: string;
      readonly port?: number;
      readonly extra_args?: readonly string[];
      readonly client?: string;
    };

/** What to start. Only the size is required. */
export interface SpawnRequest {
  readonly size: TermSize;
  readonly launcher?: Launcher;
  readonly cwd?: string;
  readonly env?: readonly (readonly [string, string])[];
}

/** A failed call into the core. `code` is stable; `message` is for people. */
export class CommandError extends Error {
  readonly code: string;
  /** What a plugin's code said of a failure of its own, where it said anything. */
  readonly data?: unknown;

  constructor(code: string, message: string, data?: unknown) {
    super(message);
    this.name = "CommandError";
    this.code = code;
    if (data !== undefined && data !== null) this.data = data;
  }
}

/** Turns whatever a failed call threw into a {@link CommandError}. */
export function toCommandError(thrown: unknown): CommandError {
  if (thrown instanceof CommandError) return thrown;
  if (typeof thrown === "object" && thrown !== null) {
    const { code, message, data } = thrown as {
      code?: unknown;
      message?: unknown;
      data?: unknown;
    };
    if (typeof code === "string" && typeof message === "string") {
      return new CommandError(code, message, data);
    }
  }
  return new CommandError("internal", typeof thrown === "string" ? thrown : String(thrown));
}

/** Whether `size` is one a terminal may have. */
export function isTermSize(size: TermSize): boolean {
  const ok = (n: number) => Number.isInteger(n) && n >= 1 && n <= MAX_DIMENSION;
  return ok(size.cols) && ok(size.rows);
}

function hex(rgb: Rgb): string {
  const part = (n: number) => (n & 0xff).toString(16).padStart(2, "0");
  return `#${part(rgb.r)}${part(rgb.g)}${part(rgb.b)}`;
}

/** The facts as the core's commands take them. */
export function factsToWire(facts: ClientFacts): unknown {
  return {
    size: { cols: facts.size.cols, rows: facts.size.rows },
    cell_px: { w: facts.cellPx.w, h: facts.cellPx.h },
    palette: {
      fg: hex(facts.palette.fg),
      bg: hex(facts.palette.bg),
      cursor: hex(facts.palette.cursor),
      ansi: facts.palette.ansi.map(hex),
    },
    scheme: facts.scheme,
    focused: facts.focused,
    visible: facts.visible,
  };
}

/** The spawn request as the core's commands take it. */
export function spawnToWire(request: SpawnRequest): unknown {
  return {
    size: { cols: request.size.cols, rows: request.size.rows },
    launcher: request.launcher ?? null,
    cwd: request.cwd ?? null,
    env: request.env ?? [],
  };
}

interface WireCaps {
  persistent: boolean;
  multi_client: boolean;
  server_layout: boolean;
  responder: Responder;
  hydrate: { snapshot: boolean; vt_replay: boolean };
}

/** Reads the capabilities the core reports. */
export function capsFromWire(wire: unknown): BackendCaps {
  const caps = wire as WireCaps;
  return {
    persistent: caps.persistent,
    multiClient: caps.multi_client,
    serverLayout: caps.server_layout,
    responder: caps.responder,
    hydrate: { snapshot: caps.hydrate.snapshot, vtReplay: caps.hydrate.vt_replay },
  };
}

interface WireListed {
  backend: string;
  info: {
    id: string;
    program: string;
    pid: number | null;
    size: TermSize;
    created_unix_ms: number;
    attached: boolean;
    exit: ExitStatus | null;
  };
}

/** Reads one entry of the core's session list. */
export function sessionFromWire(wire: unknown): SessionInfo {
  const { backend, info } = wire as WireListed;
  return {
    backend,
    id: info.id,
    program: info.program,
    pid: info.pid,
    size: info.size,
    createdUnixMs: info.created_unix_ms,
    attached: info.attached,
    exit: info.exit,
  };
}
