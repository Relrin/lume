/**
 * What the core may ask the page, and how the page answers.
 *
 * The core passes on what only the page can do or know: how large a pane that is about
 * to be made will be, running a command that acts on a terminal, reading what is on a
 * pane's screen, waiting for text to appear there, and giving a terminal a size of the
 * caller's choosing. A request arrives as a control message on the carrier; the answer
 * goes back through a command, with the request's id.
 *
 * Nothing here touches a terminal itself. The page hands in the few operations a
 * request can lead to, so the reading of requests and the shape of answers can be tested
 * without a window.
 */

import type { Intent } from "../layout/types.ts";
import { CommandError, isTermSize, type TermSize } from "../transport/types.ts";

/**
 * The terminal a request is about: a pane, the pane that shows a session, or, with
 * neither, the pane that has the keyboard.
 */
export interface PaneRef {
  readonly pane?: string;
  readonly session?: string;
}

/** The visible screen of a pane's terminal. Columns and rows count from zero. */
export interface Screen {
  readonly pane: string;
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
  readonly pane: string;
  readonly session: string;
  readonly row: number;
  readonly line: string;
}

/**
 * The operations a request can lead to. Each fails with a {@link CommandError} whose
 * code the caller can match on.
 */
export interface RemoteTarget {
  /** The grid the pane an intent would make is going to have. */
  estimate(intent: Intent): TermSize;
  /** Runs a command that is carried out in the page. */
  command(id: string, args: unknown): Promise<void>;
  screen(target: PaneRef): Screen | Promise<Screen>;
  waitText(target: PaneRef, text: string, timeoutMs: number): Promise<Match>;
  resizeTo(target: PaneRef, size: TermSize): Promise<TermSize>;
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

function refOf(body: Fields): PaneRef {
  const name = (field: string): string | undefined => {
    const value = body[field];
    if (value === undefined || value === null) return undefined;
    if (typeof value !== "string") throw badRequest(`the ${field} is not an id`);
    return value;
  };
  const pane = name("pane");
  const session = name("session");
  return {
    ...(pane === undefined ? {} : { pane }),
    ...(session === undefined ? {} : { session }),
  };
}

function sizeOf(value: unknown): TermSize {
  if (isFields(value)) {
    const size = { cols: value["cols"], rows: value["rows"] } as TermSize;
    if (isTermSize(size)) return size;
  }
  throw badRequest("the size is not one a terminal may have");
}

function wire(size: TermSize): { cols: number; rows: number } {
  return { cols: size.cols, rows: size.rows };
}

async function carryOut(target: RemoteTarget, body: Fields): Promise<Record<string, unknown>> {
  switch (body["type"]) {
    case "estimate": {
      const intent = body["intent"];
      if (!isFields(intent) || typeof intent["type"] !== "string") {
        throw badRequest("the request names no intent");
      }
      return { type: "estimated", size: wire(target.estimate(intent as unknown as Intent)) };
    }
    case "command": {
      const id = body["id"];
      if (typeof id !== "string" || id === "") throw badRequest("the request names no command");
      await target.command(id, body["args"] ?? null);
      return { type: "ran", command: id };
    }
    case "screen": {
      const screen = await target.screen(refOf(body));
      return {
        type: "screen",
        pane: screen.pane,
        session: screen.session,
        size: wire(screen.size),
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
      const match = await target.waitText(refOf(body), text, timeoutMs);
      return {
        type: "matched",
        pane: match.pane,
        session: match.session,
        row: match.row,
        line: match.line,
      };
    }
    case "resize": {
      const size = await target.resizeTo(refOf(body), sizeOf(body["size"]));
      return { type: "resized", size: wire(size) };
    }
    default:
      throw badRequest(`the page knows no request called ${String(body["type"])}`);
  }
}

/**
 * Carries out one request from the core and returns its answer. A message that is not a
 * request with an id cannot be answered, and yields nothing.
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
