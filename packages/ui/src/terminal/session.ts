/**
 * One terminal and the session it shows: the engine, the screen it reads for drawing,
 * and the attachment that carries bytes between them.
 *
 * Nothing here touches the page. Output is written into the engine as it arrives, not
 * when a frame is drawn: a hidden window draws nothing, and a view that waited for a
 * frame before taking output would stop granting credit and bring its program to a halt.
 */

import {
  RenderState,
  type ClipboardLocation,
  type CursorStyle,
  type Terminal,
  type TerminalColors,
  type Vt,
} from "@lume/vt";

import { OSC52_WRITE_MAX_BYTES, SCROLLBACK_MAX_BYTES, SCROLLBACK_MAX_LINES } from "../limits.ts";
import type { Attachment, StreamFrame } from "../transport/client.ts";
import type { CellPx, ClientFacts, ColorScheme, ExitStatus, TermSize } from "../transport/types.ts";

/** What a session tells whoever shows it. Each is optional. */
export interface SessionHooks {
  /** The screen may look different: ask for a frame. */
  changed?(): void;
  /** The terminal's grid changed size. */
  resized?(size: TermSize): void;
  /** The program ended. */
  exited?(status: ExitStatus): void;
  /** The program set its title. Not sanitised. */
  title?(title: string): void;
  bell?(): void;
  /** The program asked to put text on a clipboard, and the text is within the size cap. */
  clipboardWrite?(text: string, location: ClipboardLocation): void;
}

/** How the terminal is set up. */
export interface SessionOptions {
  readonly colors: TerminalColors;
  readonly scheme: ColorScheme;
  /** The size of one cell in device pixels, for programs that ask. */
  readonly cellPx: CellPx;
  /** How many lines that scrolled off the top are kept. Left out, the most allowed. */
  readonly scrollbackLines?: number;
  /** The cursor's shape and whether it blinks, until the program says otherwise. */
  readonly cursor?: { readonly style: CursorStyle; readonly blink: boolean };
  readonly hooks?: SessionHooks;
}

export class TerminalSession {
  readonly terminal: Terminal;
  readonly state: RenderState;
  readonly #attachment: Attachment;
  readonly #hooks: SessionHooks;
  /** The size last asked for, so the same one is not asked for twice. */
  #requested: { size: TermSize; cellPx: CellPx };
  #exit: ExitStatus | undefined;
  #disposed = false;

  constructor(vt: Vt, attachment: Attachment, options: SessionOptions) {
    this.#attachment = attachment;
    this.#hooks = options.hooks ?? {};
    this.#requested = { size: attachment.size, cellPx: options.cellPx };
    this.terminal = vt.createTerminal(attachment.size.cols, attachment.size.rows, {
      colors: options.colors,
      colorScheme: options.scheme,
      cellWidthPx: options.cellPx.w,
      cellHeightPx: options.cellPx.h,
      scrollbackMaxLines: Math.min(
        Math.max(0, options.scrollbackLines ?? SCROLLBACK_MAX_LINES),
        SCROLLBACK_MAX_LINES,
      ),
      scrollbackMaxBytes: SCROLLBACK_MAX_BYTES,
      ...(options.cursor
        ? { defaultCursorStyle: options.cursor.style, defaultCursorBlink: options.cursor.blink }
        : {}),
      resizePullsScrollback: attachment.resizePullsScrollback,
      clipboardWriteMaxBytes: OSC52_WRITE_MAX_BYTES,
      handlers: {
        output: (bytes, kind) => {
          // A program's question must get one answer. When the backend answers for
          // the session, this terminal's answers are dropped; what the user did is
          // always sent.
          if (kind === "reply" && attachment.caps.responder !== "client") return;
          this.send(bytes);
        },
        bell: () => this.#hooks.bell?.(),
        title: (title) => this.#hooks.title?.(title),
        clipboardWrite: (text, location) => this.#hooks.clipboardWrite?.(text, location),
      },
    });
    this.state = new RenderState(this.terminal);
  }

  /** Starts the flow of output. */
  start(): void {
    this.#attachment.start((frame) => this.#receive(frame));
  }

  /** How the program ended, once it has. */
  get exit(): ExitStatus | undefined {
    return this.#exit;
  }

  /** Sends bytes to the program, as if typed. Dropped once the program has ended. */
  send(bytes: Uint8Array): void {
    if (this.#disposed || this.#exit !== undefined || bytes.length === 0) return;
    this.#attachment.input(bytes);
  }

  /**
   * Asks for the terminal to have `size`. The engine's grid changes when the session
   * says, at the point of the output where the program saw the new size, so that what
   * the program drew for the old size is not reflowed into the new one.
   */
  requestSize(size: TermSize, cellPx: CellPx): void {
    if (this.#disposed) return;
    const last = this.#requested;
    if (
      last.size.cols === size.cols &&
      last.size.rows === size.rows &&
      last.cellPx.w === cellPx.w &&
      last.cellPx.h === cellPx.h
    ) {
      return;
    }
    this.#requested = { size, cellPx };
    if (this.#exit === undefined) {
      this.#attachment.resize(size, cellPx);
    } else {
      // Nobody is left to answer; what remains on screen simply follows the view.
      this.#resize(size);
    }
  }

  /** Tells the session how the view has changed. */
  facts(facts: ClientFacts): void {
    if (!this.#disposed && this.#exit === undefined) this.#attachment.facts(facts);
  }

  /** Detaches and frees the terminal. The session itself carries on. */
  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#attachment.detach();
    this.state.free();
    this.terminal.free();
  }

  #receive(frame: StreamFrame): void {
    if (this.#disposed) return;
    switch (frame.kind) {
      case "data":
        this.terminal.write(frame.bytes);
        // Only now are the bytes out of the way, and more may come.
        this.#attachment.consumed(frame.bytes.length);
        this.#hooks.changed?.();
        break;
      case "resized":
        this.#resize(frame.size);
        break;
      case "hydrate":
        // Output to replay rebuilds the screen. A serialised terminal is for an engine
        // that can load one, which this one is not asked to do yet.
        if (frame.form === "vt_replay") {
          this.terminal.write(frame.bytes);
          this.#hooks.changed?.();
        }
        break;
      case "event":
        // The terminal reads titles and bells out of the output itself.
        break;
      case "exit":
        this.#exit = frame.status;
        this.#hooks.exited?.(frame.status);
        this.#hooks.changed?.();
        break;
    }
  }

  #resize(size: TermSize): void {
    const { cellPx } = this.#requested;
    this.terminal.resize(size.cols, size.rows, cellPx.w, cellPx.h);
    this.#hooks.resized?.(size);
    this.#hooks.changed?.();
  }
}
