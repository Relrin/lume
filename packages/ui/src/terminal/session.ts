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
import type {
  CellPx,
  ClientFacts,
  ColorScheme,
  ExitStatus,
  SessionEvent,
  TermSize,
} from "../transport/types.ts";

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
  /** The state the session's screen was in when the attachment opened. */
  readonly start?: HydrateFrame;
  /**
   * Keep only this many of the newest lines of scrollback here, and fetch older ones
   * from the backend when they are wanted. Left out, or with a backend that cannot send
   * its screen, the terminal keeps `scrollbackLines` itself.
   */
  readonly windowLines?: number;
}

export class TerminalSession {
  #terminal: Terminal;
  #state: RenderState;
  readonly #vt: Vt;
  readonly #settings: TerminalOptions;
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
    const settings: TerminalOptions = {
      colors: options.colors,
      colorScheme: options.scheme,
      cellWidthPx: options.cellPx.w,
      cellHeightPx: options.cellPx.h,
      scrollbackMaxLines: this.#allLines,
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
    };
    this.#settings = settings;
    this.#terminal = startFrom(vt, attachment, this.#withLines(this.#windowLines), options.start);
    this.#state = new RenderState(this.#terminal);
    this.#cut = this.#terminal.leftHistoryOut;
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
    if (this.#exit !== undefined) return;
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
    this.#released = true;
    this.#restoring = false;
    this.#fetching = false;
    this.#attachment.release();
    const old = this.#terminal;
    this.#terminal = this.#vt.createTerminal(old.cols, old.rows, this.#withLines(0));
    this.#state.free();
    this.#state = new RenderState(this.#terminal);
    old.free();
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
        this.#terminal.write(frame.bytes);
        startOutput();
        // Only now are the bytes out of the way, and more may come.
        this.#attachment.consumed(frame.bytes.length);
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
    const all = this.#fetching || this.#holdsAll;
    this.#fetching = false;
    const settings = this.#withLines(all ? undefined : this.#windowLines);
    const old = this.#terminal;
    const { cols, rows } = old;
    let next: Terminal | undefined;
    if (frame.form === "snapshot" && frame.vtVersion === this.#vt.version) {
      try {
        next = this.#vt.loadTerminal(frame.bytes, {
          ...settings,
          ...(all ? {} : { historyLines: this.#windowLines }),
        });
      } catch (error) {
        console.warn("the copy of the screen could not be loaded; it was not used", error);
        return;
      }
    } else if (frame.form === "vt_replay") {
      next = this.#vt.createTerminal(cols, rows, settings);
      next.write(frame.bytes);
    } else {
      console.warn(`the copy of the screen is of engine ${frame.vtVersion}; it was not used`);
      return;
    }
    // The view stays on the lines it was showing: as far from the newest as it was.
    const before = old.scrollbar();
    const fromEnd = before.total - before.offset - before.length;
    if (next.cols !== cols || next.rows !== rows) next.resize(cols, rows);
    const after = next.scrollbar();
    if (fromEnd > 0) next.scrollToRow(Math.max(0, after.total - after.length - fromEnd));
    this.#terminal = next;
    this.#state.free();
    this.#state = new RenderState(next);
    old.free();
    this.#holdsAll = all;
    this.#cut = !all && next.leftHistoryOut;
    this.#hooks.replaced?.();
    this.#hooks.changed?.();
  }

  #resize(size: TermSize): void {
    const { cellPx } = this.#requested;
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

/**
 * The terminal a session starts with: loaded from the snapshot it was sent when the
 * snapshot is of this engine's version, rebuilt from the output it was sent to replay,
 * or blank.
 */
function startFrom(
  vt: Vt,
  attachment: Attachment,
  settings: TerminalOptions,
  start: HydrateFrame | undefined,
): Terminal {
  const { cols, rows } = attachment.size;
  if (start?.form === "snapshot") {
    if (start.vtVersion === vt.version) {
      try {
        // Only as much history as the terminal keeps is unpacked.
        const loaded = vt.loadTerminal(start.bytes, {
          ...settings,
          ...(settings.scrollbackMaxLines === undefined
            ? {}
            : { historyLines: settings.scrollbackMaxLines }),
        });
        // The session's size is the one the output that follows is drawn for.
        if (loaded.cols !== cols || loaded.rows !== rows) loaded.resize(cols, rows);
        return loaded;
      } catch (error) {
        console.warn("the saved screen could not be loaded; starting blank", error);
      }
    } else {
      console.warn(
        `the saved screen is of engine ${start.vtVersion}, not ${vt.version}; starting blank`,
      );
    }
    return vt.createTerminal(cols, rows, settings);
  }
  const terminal = vt.createTerminal(cols, rows, settings);
  if (start?.form === "vt_replay") terminal.write(start.bytes);
  return terminal;
}
