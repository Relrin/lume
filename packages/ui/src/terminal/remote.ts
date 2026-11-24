/**
 * What the core may ask the page, and how the page answers.
 *
 * The core passes on what only the page can do or know: how large a pane that is about
 * to be made will be, running a command that acts on a terminal, reading what is on a
 * pane's screen, waiting for text to appear there, giving a terminal a size of the
 * caller's choosing, and putting a plugin's question, or one of the SSH client's, to the
 * user. A request arrives as a control message on the carrier; the answer goes back
 * through a command, with the request's id.
 *
 * Nothing here touches a terminal itself. The page hands in the few operations a
 * request can lead to, so the reading of requests and the shape of answers can be tested
 * without a window.
 */

import type { Intent } from "../layout/types.ts";
import { CommandError, isTermSize, type TermSize } from "../transport/types.ts";
import type { PickItem, PluginAsk, PluginAsker, SignIn, SignInKind } from "./prompts.ts";

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
  /** Puts a plugin's question to the user and gives their answer. */
  ask(ask: PluginAsk): Promise<Record<string, unknown>>;
  /** Puts a question of the SSH client's to the user and gives what they answer, if they do. */
  signIn(signIn: SignIn): Promise<string | null>;
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

function text(body: Fields, field: string): string {
  const value = body[field];
  if (typeof value !== "string") throw badRequest(`the question has no ${field}`);
  return value;
}

function maybeText(body: Fields, field: string): string | null {
  const value = body[field];
  return typeof value === "string" ? value : null;
}

/** A plugin's question as the core sent it, checked field by field. */
export function readAsk(body: unknown): PluginAsk {
  if (!isFields(body)) throw badRequest("the question is not an object");
  const from = body["plugin"];
  if (!isFields(from)) throw badRequest("the question names no plugin");
  const plugin: PluginAsker = { id: text(from, "id"), name: text(from, "name") };
  switch (body["kind"]) {
    case "permission":
      return {
        kind: "permission",
        plugin,
        permission: text(body, "permission"),
        what: text(body, "what"),
        warning: maybeText(body, "warning"),
        reason: maybeText(body, "reason") ?? "",
      };
    case "each":
      return {
        kind: "each",
        plugin,
        permission: text(body, "permission"),
        text: text(body, "text"),
        place: maybeText(body, "place"),
      };
    case "confirm":
      return {
        kind: "confirm",
        plugin,
        title: text(body, "title"),
        message: text(body, "message"),
        confirm: text(body, "confirm"),
        danger: body["danger"] === true,
      };
    case "pick": {
      const items = body["items"];
      if (!Array.isArray(items)) throw badRequest("the choice has no items");
      const read: PickItem[] = items.map((item: unknown) => {
        if (!isFields(item)) throw badRequest("an item of the choice is not an object");
        return {
          id: text(item, "id"),
          label: text(item, "label"),
          detail: maybeText(item, "detail"),
        };
      });
      return {
        kind: "pick",
        plugin,
        title: maybeText(body, "title"),
        placeholder: maybeText(body, "placeholder"),
        items: read,
      };
    }
    default:
      throw badRequest(`the page knows no question called ${String(body["kind"])}`);
  }
}

const SIGN_IN_KINDS: readonly SignInKind[] = [
  "host_key",
  "password",
  "passphrase",
  "confirm",
  "code",
];

/** A question of the SSH client's as the core sent it, checked field by field. */
export function readSignIn(body: Readonly<Record<string, unknown>>): SignIn {
  const { ask, again } = body;
  const kind = SIGN_IN_KINDS.find((known) => known === body["kind"]);
  if (kind === undefined) {
    throw badRequest(`the page knows no question called ${String(body["kind"])}`);
  }
  if (typeof ask !== "number") throw badRequest("the question has no number");
  if (typeof again !== "boolean") throw badRequest("the question has no again");
  return {
    ask,
    kind,
    host: text(body, "host"),
    address: text(body, "address"),
    words: text(body, "words"),
    again,
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
    case "plugin_ask": {
      const answer = await target.ask(readAsk(body["ask"]));
      return { type: "answered", answer };
    }
    case "sign_in":
      return { type: "secret", secret: await target.signIn(readSignIn(body)) };
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
