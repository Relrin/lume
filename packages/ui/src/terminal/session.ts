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
  type TerminalHandlers,
  type TerminalOptions,
  type Vt,
} from "@lume/vt";

import { startOutput } from "../start-marks.ts";
import { HISTORY_FULL_SHARE, LETTERBOX_SETTLE_MS } from "../tuning.ts";
import {
  OSC52_WRITE_MAX_BYTES,
  SCROLLBACK_DEFAULT_LINES,
  SCROLLBACK_MAX_BYTES,
  SCROLLBACK_MAX_LINES,
} from "../limits.ts";
import type { Attachment, HydrateFrame, StreamFrame } from "../transport/client.ts";
import {
  withinCells,
  type CellPx,
  type ClientFacts,
  type ColorScheme,
  type ExitStatus,
  type SessionEvent,
  type TermSize,
} from "../transport/types.ts";
import type { ServedShare } from "./budget.ts";

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
  /**
   * The session reported something besides its output: what its shell runs, where it
   * is, which program has the terminal, how the session ended.
   */
  reported?(event: SessionEvent): void;
  /** The program asked to put text on a clipboard, and the text is within the size cap. */
  clipboardWrite?(text: string, location: ClipboardLocation): void;
  /**
   * The session's terminal was replaced by one brought up to date from the backend:
   * whatever holds on to the old one must take the new one.
   */
  replaced?(): void;
  /**
   * Whether the terminal has a size other than the one this view asked for, and has
   * kept it: another view of the same session decides its size.
   */
  sizedElsewhere?(sizedElsewhere: boolean): void;
  /** The session's {@link TerminalSession.problem} changed. */
  problem?(problem: string): void;
}

/** What is said of a copy of the screen that cannot be shown, and of the older lines. */
const NOT_SHOWN = "What was on this terminal's screen before could not be shown.";
const NOT_FETCHED = "The older lines of this terminal could not be shown.";

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
  /** The state the session's screen was in when the attachment opened. */
  readonly start?: HydrateFrame;
  /**
   * Keep only this many of the newest lines of scrollback here, and fetch older ones
   * from the backend when they are wanted. Left out, or with a backend that cannot send
   * its screen, the terminal keeps `scrollbackLines` itself.
   */
  readonly windowLines?: number;
  /**
   * What the terminal may hold of the window, for a terminal of a server. It is closed
   * with the session.
   */
  readonly share?: ServedShare;
}

export class TerminalSession {
  #terminal: Terminal;
  #state: RenderState;
  readonly #vt: Vt;
  readonly #settings: TerminalOptions;
  /** What hears the terminal. A terminal has it once its screen is what the session's is. */
  readonly #handlers: TerminalHandlers;
  readonly #share: ServedShare | undefined;
  /** How many lines of scrollback the backend keeps, and so how many there can be. */
  readonly #allLines: number;
  /** How many this terminal keeps while it holds only the newest; undefined: all. */
  readonly #windowLines: number | undefined;
  /** Whether the older lines are here now, fetched from the backend. */
  #holdsAll: boolean;
  /** Whether the older lines were asked for and have not arrived. */
  #fetching = false;
  /** Whether lines older than the terminal holds were left out when it was made. */
  #cut = false;
  /** Whether the older lines came and could not be shown, and nothing has since. */
  #unfetchable = false;
  #problem = "";
  readonly #attachment: Attachment;
  readonly #hooks: SessionHooks;
  /** The size last asked for, so the same one is not asked for twice. */
  #requested: { size: TermSize; cellPx: CellPx };
  #exit: ExitStatus | undefined;
  #disposed = false;
  /** Whether the screen was let go of, and the output with it, until it is asked back. */
  #released = false;
  /** Whether the screen was asked back and has not arrived. */
  #restoring = false;
  #sizedElsewhere = false;
  #sizeTimer: ReturnType<typeof setTimeout> | undefined;

  /** A session that cannot be made throws, and has let go of its attachment. */
  constructor(vt: Vt, attachment: Attachment, options: SessionOptions) {
    this.#attachment = attachment;
    this.#hooks = options.hooks ?? {};
    this.#requested = { size: attachment.size, cellPx: options.cellPx };
    this.#vt = vt;
    this.#allLines = Math.min(
      Math.max(0, options.scrollbackLines ?? SCROLLBACK_DEFAULT_LINES),
      SCROLLBACK_MAX_LINES,
    );
    const canFetch = attachment.caps.hydrate.snapshot || attachment.caps.hydrate.vtReplay;
    this.#windowLines =
      options.windowLines !== undefined && canFetch && options.windowLines < this.#allLines
        ? Math.max(0, options.windowLines)
        : undefined;
    this.#holdsAll = this.#windowLines === undefined;
    this.#share = options.share;
    this.#settings = {
      colors: options.colors,
      colorScheme: options.scheme,
      cellWidthPx: options.cellPx.w,
      cellHeightPx: options.cellPx.h,
      scrollbackMaxLines: this.#allLines,
      scrollbackMaxBytes: Math.min(
        options.share?.scrollbackBytes ?? SCROLLBACK_MAX_BYTES,
        SCROLLBACK_MAX_BYTES,
      ),
      ...(options.cursor
        ? { defaultCursorStyle: options.cursor.style, defaultCursorBlink: options.cursor.blink }
        : {}),
      resizePullsScrollback: attachment.resizePullsScrollback,
      clipboardWriteMaxBytes: OSC52_WRITE_MAX_BYTES,
    };
    this.#handlers = {
      output: (bytes, kind) => {
        // A program's question must get one answer. When the backend answers for the
        // session, this terminal's answers are dropped; what the user did is always
        // sent.
        if (kind === "reply" && attachment.caps.responder !== "client") return;
        this.send(bytes);
      },
      bell: () => this.#hooks.bell?.(),
      title: (title) => this.#hooks.title?.(title),
      clipboardWrite: (text, location) => this.#hooks.clipboardWrite?.(text, location),
    };
    try {
      const size = this.#fit(attachment.size);
      const { start } = options;
      let made: Made | undefined;
      if (start) {
        try {
          made = this.#make(start, size, this.#windowLines);
        } catch (error) {
          console.warn("the copy of the screen could not be shown; starting blank", error);
          this.#problem = NOT_SHOWN;
        }
      }
      made ??= this.#make(undefined, size, this.#windowLines);
      this.#terminal = made.terminal;
      this.#state = made.state;
      this.#cut = made.terminal.leftHistoryOut;
    } catch (error) {
      attachment.detach();
      this.#share?.close();
      throw error;
    }
  }

  /**
   * Makes a terminal from a copy of a screen, or a blank one, and what reads it, with
   * `size` and `lines` of scrollback (all, if not given). Leaves nothing behind when it
   * cannot be done.
   *
   * Nothing hears the terminal while it is made. What a copy holds was asked for and
   * answered when the program printed it: taken again, it would put text on the clipboard
   * each time a pane is shown, and type answers into a program that asked nothing.
   */
  #make(from: HydrateFrame | undefined, size: TermSize, lines: number | undefined): Made {
    const settings = this.#withLines(lines);
    let terminal: Terminal;
    if (from?.form === "snapshot") {
      if (from.vtVersion !== this.#vt.version) {
        throw new Error(`the copy is of engine ${from.vtVersion}, not ${this.#vt.version}`);
      }
      // Only as much history as the terminal keeps is unpacked.
      terminal = this.#vt.loadTerminal(from.bytes, {
        ...settings,
        historyLines: lines ?? this.#allLines,
      });
    } else {
      terminal = this.#vt.createTerminal(size.cols, size.rows, settings);
    }
    try {
      if (from?.form === "vt_replay") terminal.write(from.bytes);
      // The session's size is the one the output that follows is drawn for.
      if (terminal.cols !== size.cols || terminal.rows !== size.rows) {
        terminal.resize(size.cols, size.rows);
      }
      const state = new RenderState(terminal);
      terminal.handlers = this.#handlers;
      return { terminal, state };
    } catch (error) {
      terminal.free();
      throw error;
    }
  }

  /** Shows another terminal in place of the one there is, which is freed. */
  #replace(made: Made): void {
    const { state, terminal } = { state: this.#state, terminal: this.#terminal };
    this.#terminal = made.terminal;
    this.#state = made.state;
    state.free();
    terminal.free();
  }

  /** The size the terminal may have in place of one it is to have. */
  #fit(size: TermSize): TermSize {
    return this.#share ? this.#share.fit(size) : withinCells(size);
  }

  /**
   * Why what is on show is not all there is to show, in a sentence. Empty when it is.
   */
  get problem(): string {
    return this.#problem;
  }

  #setProblem(problem: string): void {
    if (problem === this.#problem) return;
    this.#problem = problem;
    this.#hooks.problem?.(problem);
  }

  /** The terminal the session is shown in. It is replaced when older lines arrive. */
  get terminal(): Terminal {
    return this.#terminal;
  }

  /** The screen as the renderer reads it, for the terminal as it is now. */
  get state(): RenderState {
    return this.#state;
  }

  /**
   * Whether older lines are fetched from the backend as they are wanted: the terminal
   * keeps only the newest itself.
   */
  get fetchesHistory(): boolean {
    return this.#windowLines !== undefined;
  }

  /** Whether every line of scrollback the backend keeps is here. */
  get holdsAllHistory(): boolean {
    return this.#holdsAll;
  }

  /**
   * Whether the terminal holds as many lines as it keeps while only the newest are
   * here, so that there may be older ones to fetch.
   */
  get historyCut(): boolean {
    if (this.#holdsAll || this.#windowLines === undefined) return false;
    if (this.#cut) return true;
    const { total, length } = this.#terminal.scrollbar();
    return total - length >= this.#windowLines * HISTORY_FULL_SHARE;
  }

  /**
   * Fetches the older lines from the backend. They arrive with the next copy of the
   * screen, which replaces the terminal; the view stays on the lines it shows.
   */
  fetchHistory(): void {
    if (this.#disposed || this.#released || this.#holdsAll || this.#fetching) return;
    if (this.#exit !== undefined || this.#unfetchable) return;
    this.#fetching = true;
    this.#attachment.rehydrate();
  }

  /**
   * Whether this session can let go of its screen and get it back: the backend keeps a
   * copy it can send.
   */
  get canRelease(): boolean {
    const { hydrate } = this.#attachment.caps;
    return hydrate.snapshot || hydrate.vtReplay;
  }

  /** Whether the screen has been let go of and not yet asked back. */
  get released(): boolean {
    return this.#released;
  }

  /** Whether the screen was asked back and has not arrived yet. */
  get restoring(): boolean {
    return this.#restoring;
  }

  /**
   * Lets go of the screen and its scrollback while nobody looks at them, and of the
   * output that would keep them up to date. What the session reports, its size and its
   * end still arrive. The terminal is replaced by an empty one of the same size, which
   * {@link restore} replaces in turn with a copy of the session's screen.
   */
  release(): void {
    if (this.#disposed || this.#released || !this.canRelease) return;
    const { cols, rows } = this.#terminal;
    const empty = this.#make(undefined, { cols, rows }, 0);
    this.#released = true;
    this.#restoring = false;
    this.#fetching = false;
    this.#attachment.release();
    this.#replace(empty);
    this.#holdsAll = this.#windowLines === undefined;
    this.#cut = false;
    this.#hooks.replaced?.();
  }

  /** Asks for the screen back after {@link release}. It arrives as a copy of the screen. */
  restore(): void {
    if (this.#disposed || !this.#released) return;
    this.#released = false;
    this.#restoring = true;
    this.#attachment.rehydrate();
  }

  /**
   * Whether the session is a server's, which keeps its own layout: what its terminal says
   * is the word of another machine.
   */
  get served(): boolean {
    return this.#attachment.caps.serverLayout;
  }

  /** Whether another view of the session decides the terminal's size. */
  get sizedElsewhere(): boolean {
    return this.#sizedElsewhere;
  }

  /** Lets the older lines go again, keeping only the newest. */
  releaseHistory(): void {
    if (this.#disposed || this.#windowLines === undefined || !this.#holdsAll) return;
    this.#holdsAll = false;
    const { total, length } = this.#terminal.scrollbar();
    this.#cut = total - length > this.#windowLines;
    this.#terminal.setScrollbackLines(this.#windowLines);
    this.#hooks.changed?.();
  }

  /** The terminal's settings with a scrollback of `lines`, or of all lines. */
  #withLines(lines: number | undefined): TerminalOptions {
    return { ...this.#settings, scrollbackMaxLines: lines ?? this.#allLines };
  }

  /**
   * Whether this terminal answers the program's questions to the terminal. When the
   * backend does, what the view would report itself (focus, a change of colour scheme)
   * is the backend's to report too, from the facts the view sends it.
   */
  get answersQueries(): boolean {
    return this.#attachment.caps.responder === "client";
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
      this.#checkSize();
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
    if (this.#sizeTimer !== undefined) clearTimeout(this.#sizeTimer);
    this.#sizeTimer = undefined;
    this.#attachment.detach();
    this.#share?.close();
    this.#state.free();
    this.#terminal.free();
  }

  #receive(frame: StreamFrame): void {
    if (this.#disposed) return;
    switch (frame.kind) {
      case "data":
        // Output on its way when the screen was let go of is not wanted, but it was
        // sent against credit and is granted back as if it had been taken.
        if (this.#released) {
          this.#attachment.consumed(frame.bytes.length);
          break;
        }
        try {
          this.#terminal.write(frame.bytes);
          startOutput();
        } finally {
          // Only now are the bytes out of the way, and more may come: also when
          // something that heard them failed, or the program would stand still.
          this.#attachment.consumed(frame.bytes.length);
        }
        this.#hooks.changed?.();
        break;
      case "resized":
        this.#resize(frame.size);
        break;
      case "hydrate":
        this.#restart(frame);
        break;
      case "event":
        // The terminal reads titles and bells out of the output itself. What else a
        // session reports is for whoever shows more than the terminal.
        this.#hooks.reported?.(frame.event);
        break;
      case "exit":
        this.#exit = frame.status;
        this.#checkSize();
        this.#hooks.exited?.(frame.status);
        this.#hooks.changed?.();
        break;
    }
  }

  /**
   * Starts again from a copy of the screen sent in the middle of the flow: the output
   * before it is part of it, the output after it follows on from it. When the older lines
   * were asked for, the copy is taken with all of them.
   */
  #restart(frame: HydrateFrame): void {
    // One asked for before the screen was let go of; another is asked for when it is
    // wanted again.
    if (this.#released) return;
    this.#restoring = false;
    const fetching = this.#fetching;
    const all = fetching || this.#holdsAll;
    this.#fetching = false;
    const { cols, rows } = this.#terminal;
    let made: Made;
    try {
      made = this.#make(frame, { cols, rows }, all ? undefined : this.#windowLines);
    } catch (error) {
      console.warn("the copy of the screen could not be shown; it was not used", error);
      // What was asked for once and cannot be shown is not asked for at every frame.
      this.#unfetchable = fetching;
      this.#setProblem(fetching ? NOT_FETCHED : NOT_SHOWN);
      // What there is gets drawn: the view may have waited for this.
      this.#hooks.changed?.();
      return;
    }
    // The view stays on the lines it was showing: as far from the newest as it was.
    const before = this.#terminal.scrollbar();
    const fromEnd = before.total - before.offset - before.length;
    const after = made.terminal.scrollbar();
    if (fromEnd > 0) {
      made.terminal.scrollToRow(Math.max(0, after.total - after.length - fromEnd));
    }
    this.#replace(made);
    this.#holdsAll = all;
    this.#cut = !all && made.terminal.leftHistoryOut;
    this.#unfetchable = false;
    this.#setProblem("");
    this.#hooks.replaced?.();
    this.#hooks.changed?.();
  }

  #resize(asked: TermSize): void {
    const { cellPx } = this.#requested;
    const size = this.#fit(asked);
    this.#terminal.resize(size.cols, size.rows, cellPx.w, cellPx.h);
    this.#checkSize();
    this.#hooks.resized?.(size);
    this.#hooks.changed?.();
  }

  /**
   * Notices a terminal whose size stays other than the one asked for. Only a backend
   * that shows a session in several views answers a request with another size, and
   * only once the answer has had time to come back does it count.
   */
  #checkSize(): void {
    const differs = (): boolean => {
      if (this.#disposed || this.#exit !== undefined) return false;
      if (!this.#attachment.caps.multiClient) return false;
      const wanted = this.#requested.size;
      return wanted.cols !== this.#terminal.cols || wanted.rows !== this.#terminal.rows;
    };
    if (!differs()) {
      if (this.#sizeTimer !== undefined) clearTimeout(this.#sizeTimer);
      this.#sizeTimer = undefined;
      this.#setSizedElsewhere(false);
      return;
    }
    if (this.#sizedElsewhere || this.#sizeTimer !== undefined) return;
    this.#sizeTimer = setTimeout(() => {
      this.#sizeTimer = undefined;
      if (differs()) this.#setSizedElsewhere(true);
    }, LETTERBOX_SETTLE_MS);
  }

  #setSizedElsewhere(sizedElsewhere: boolean): void {
    if (this.#sizedElsewhere === sizedElsewhere) return;
    this.#sizedElsewhere = sizedElsewhere;
    this.#hooks.sizedElsewhere?.(sizedElsewhere);
  }
}

/** A terminal and what reads its screen. */
interface Made {
  readonly terminal: Terminal;
  readonly state: RenderState;
}
