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

/** Something a session noticed in its program's output. */
export type SessionEvent =
  | { readonly type: "title"; readonly title: string }
  | { readonly type: "cwd"; readonly url: string }
  | { readonly type: "bell" };

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

  constructor(code: string, message: string) {
    super(message);
    this.name = "CommandError";
    this.code = code;
  }
}

/** Turns whatever a failed call threw into a {@link CommandError}. */
export function toCommandError(thrown: unknown): CommandError {
  if (thrown instanceof CommandError) return thrown;
  if (typeof thrown === "object" && thrown !== null) {
    const { code, message } = thrown as { code?: unknown; message?: unknown };
    if (typeof code === "string" && typeof message === "string") {
      return new CommandError(code, message);
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
