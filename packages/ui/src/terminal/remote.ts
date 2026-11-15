/**
 * What the core may ask the page, and how the page answers.
 *
 * The core passes on what only the page can do or know: starting a session in the window,
 * reading what is on a session's screen, waiting for text to appear there, and giving the
 * terminal a size of the caller's choosing. A request arrives as a control message on the
 * carrier; the answer goes back through a command, with the request's id.
 *
 * Nothing here touches the terminal itself. The page hands in the few operations a
 * request can lead to, so the reading of requests and the shape of answers can be tested
 * without a window.
 */

import { CommandError, isTermSize, type Launcher, type TermSize } from "../transport/types.ts";

/** What to start in the window. Everything left out is chosen for the caller. */
export interface OpenSpec {
  readonly launcher?: Launcher;
  readonly cwd?: string;
  readonly env?: readonly (readonly [string, string])[];
  /** The terminal's size. Left out, it is the size that fits the window. */
  readonly size?: TermSize;
}

/** A session that was started and shown, and the one it took the place of. */
export interface Opened {
  readonly session: string;
  /** The session shown before, which keeps running. */
  readonly previous: string | null;
}

/** The visible screen of a session. Columns and rows count from zero. */
export interface Screen {
  readonly session: string;
  readonly size: TermSize;
  readonly cursor: { readonly col: number; readonly row: number; readonly visible: boolean };
  /** Whether a full-screen program has switched to the alternate screen. */
  readonly alternate: boolean;
  /** One string for each row, without the blanks at its end. */
  readonly lines: readonly string[];
}

/** Where text that was waited for is. */
export interface Match {
  readonly session: string;
  readonly row: number;
  readonly line: string;
}

/**
 * The operations a request can lead to. Each takes the session the request named, or
 * nothing for the one the window shows, and fails with a {@link CommandError} whose code
 * the caller can match on.
 */
export interface RemoteTarget {
  open(spec: OpenSpec): Promise<Opened>;
  screen(session: string | undefined): Screen;
  waitText(session: string | undefined, text: string, timeoutMs: number): Promise<Match>;
  resizeTo(session: string | undefined, size: TermSize): Promise<TermSize>;
}

/** An answer as the core takes it: what the request yielded, or why it yielded nothing. */
export type RemoteResult =
  | { readonly ok: Record<string, unknown> }
  | { readonly err: { readonly code: string; readonly message: string } };

/** The answer to one request, with the id that says which. */
export interface RemoteAnswer {
  readonly id: number;
  readonly result: RemoteResult;
}

type Fields = Record<string, unknown>;

function isFields(value: unknown): value is Fields {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function badRequest(message: string): CommandError {
  return new CommandError("bad_request", message);
}

function sessionOf(body: Fields): string | undefined {
  const session = body["session"];
  if (session === undefined || session === null) return undefined;
  if (typeof session !== "string") throw badRequest("the session is not an id");
  return session;
}

function sizeOf(value: unknown): TermSize {
  if (isFields(value)) {
    const size = { cols: value["cols"], rows: value["rows"] } as TermSize;
    if (isTermSize(size)) return size;
  }
  throw badRequest("the size is not one a terminal may have");
}

function specOf(value: unknown): OpenSpec {
  if (!isFields(value)) throw badRequest("the request says nothing about what to start");
  const { launcher, cwd, env, size } = value;
  return {
    ...(launcher === undefined || launcher === null ? {} : { launcher: launcher as Launcher }),
    ...(typeof cwd === "string" ? { cwd } : {}),
    ...(Array.isArray(env) ? { env: env as [string, string][] } : {}),
    ...(size === undefined || size === null ? {} : { size: sizeOf(size) }),
  };
}

async function carryOut(target: RemoteTarget, body: Fields): Promise<Record<string, unknown>> {
  switch (body["type"]) {
    case "open": {
      const opened = await target.open(specOf(body["spec"]));
      return { type: "spawned", session: opened.session, previous: opened.previous };
    }
    case "screen": {
      const screen = target.screen(sessionOf(body));
      return {
        type: "screen",
        session: screen.session,
        size: { cols: screen.size.cols, rows: screen.size.rows },
        cursor: {
          col: screen.cursor.col,
          row: screen.cursor.row,
          visible: screen.cursor.visible,
        },
        alternate: screen.alternate,
        lines: [...screen.lines],
      };
    }
    case "wait_text": {
      const text = body["text"];
      const timeoutMs = body["timeout_ms"];
      if (typeof text !== "string" || text === "") throw badRequest("there is no text to wait for");
      if (typeof timeoutMs !== "number" || !(timeoutMs >= 0)) {
        throw badRequest("the wait has no limit");
      }
      const match = await target.waitText(sessionOf(body), text, timeoutMs);
      return { type: "matched", session: match.session, row: match.row, line: match.line };
    }
    case "resize": {
      const size = await target.resizeTo(sessionOf(body), sizeOf(body["size"]));
      return { type: "resized", size: { cols: size.cols, rows: size.rows } };
    }
    default:
      throw badRequest(`the page knows no request called ${String(body["type"])}`);
  }
}

/**
 * Carries out one control message from the core and returns its answer. A message that is
 * not a request with an id cannot be answered, and yields nothing.
 */
export async function answerRequest(
  target: RemoteTarget,
  message: unknown,
): Promise<RemoteAnswer | undefined> {
  const request = isFields(message) ? message["request"] : undefined;
  if (!isFields(request)) return undefined;
  const { id, body } = request;
  if (typeof id !== "number") return undefined;
  try {
    if (!isFields(body)) throw badRequest("the request has no body");
    return { id, result: { ok: await carryOut(target, body) } };
  } catch (thrown) {
    const code = thrown instanceof CommandError ? thrown.code : "internal";
    const message = thrown instanceof Error ? thrown.message : String(thrown);
    return { id, result: { err: { code, message } } };
  }
}
