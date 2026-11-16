/**
 * One terminal on the page: the part that touches the DOM.
 *
 * A view is handed a few elements once and from then on works on them directly. Output,
 * frames, the cursor, the selection and scrolling never pass through the component tree:
 * a terminal changes many times a second, and none of that is anyone else's business.
 *
 * The view owns nothing that decides behaviour. Which bytes a key sends is the engine's;
 * what a key event means, what the pointer is for, what counts as a link and what may be
 * pasted are small modules beside this one, written without the DOM so they can be
 * tested without it. What is left here is wiring.
 *
 * A view outlives its elements. A pane in a tab that is not on show has no elements and
 * draws nothing, but its terminal still takes its session's output, so the program is
 * never held up and the screen is there the moment the tab is shown again.
 */

import type { CellRenderer, FrameView, RenderHost } from "@lume/renderer";
import {
  clearSelection,
  encodeColorScheme,
  encodeFocus,
  encodePaste,
  extendSelection,
  hasSelection,
  hyperlinkAt,
  isPasteSafe,
  Mode,
  MouseEncoder,
  newHighlights,
  Search,
  selectAll,
  SelectionGesture,
  selectionText,
  type Autoscroll,
  type CursorStyle,
  type FrameCursor,
  type KeyEncoder,
  type KeyInput,
  type MouseButton,
  type SearchStatus,
  type Vt,
} from "@lume/vt";

import { LINK_SCHEMES_TO_CONFIRM, PASTE_MAX_BYTES } from "../limits.ts";
import {
  cssColor,
  factsPalette,
  rendererTheme,
  terminalColors,
  type Appearance,
} from "../theme.ts";
import type { TransportClient } from "../transport/client.ts";
import {
  CommandError,
  MAX_DIMENSION,
  type CellPx,
  type ClientFacts,
  type TermSize,
} from "../transport/types.ts";
import {
  AUTOSCROLL_INTERVAL_MS,
  CLICK_REPEAT_DISTANCE_CELLS,
  CLICK_REPEAT_INTERVAL_MS,
  CURSOR_BLINK_INTERVAL_MS,
  FONT_SIZE_MAX_PX,
  FONT_SIZE_MIN_PX,
  MAC_OPTION_AS_ALT,
  PINNED_SIZE_TIMEOUT_MS,
  REPORT_INTERVAL_MS,
  SEARCH_CLEAR_ROWS,
  SEARCH_MATCHES_MAX,
  SEARCH_REFEED_INTERVAL_MS,
} from "../tuning.ts";
import {
  pasteDecision,
  pastePreview,
  ProgramClipboard,
  type ClipboardService,
} from "./clipboard.ts";
import {
  isModifierKey,
  mapKey,
  mapKeyRelease,
  textInput,
  type KeyLayout,
  type KeyMapping,
  type Platform,
} from "./keyboard.ts";
import { systemKey, unboundSystemKey, type TerminalAction } from "./keymap.ts";
import { addressUnder, linkScheme, markedRun, type LinkHit } from "./links.ts";
import {
  gridPoint,
  mouseButton,
  pointerModifiers,
  pointerRoute,
  WheelAccumulator,
  wheelRoute,
  type GridPoint,
} from "./mouse.ts";
import type { PromptStore } from "./prompts.ts";
import { TerminalSession } from "./session.ts";
import { sanitizeTitle } from "./title.ts";

/** The elements a view works on. They are created once and never replaced. */
export interface ViewElements {
  /** The whole terminal; its size is the terminal's size. */
  readonly root: HTMLElement;
  /** Where the renderer keeps its canvas. Holds nothing else. */
  readonly surface: HTMLElement;
  /** Takes the keyboard. Invisible, and kept at the cursor for the input method's sake. */
  readonly input: HTMLTextAreaElement;
  /** The moving part of the scrollbar. */
  readonly scrollThumb: HTMLElement;
  /** Shows text an input method is still composing. */
  readonly preedit: HTMLElement;
  /**
   * One line of text about the terminal, for what the terminal cannot show itself:
   * where a link leads, or why its session could not be started.
   */
  readonly status: HTMLElement;
}

/** The cursor a terminal has until the program in it chooses one. */
export interface CursorDefault {
  readonly style: CursorStyle;
  readonly blink: boolean;
}

/** How a view is set up, and may be set up again while it runs. */
export interface ViewSettings {
  readonly appearance: Appearance;
  readonly cursor: CursorDefault;
  /** How many lines that scrolled off the top a terminal keeps. */
  readonly scrollbackLines: number;
  /** How much larger than the look's own the text is, in CSS pixels; negative for smaller. */
  readonly zoomPx: number;
}

/** What a view needs from its surroundings. */
export interface ViewServices {
  readonly vt: Vt;
  readonly host: RenderHost;
  readonly keys: KeyEncoder;
  readonly platform: Platform;
  readonly layout: KeyLayout;
  readonly clipboard: ClipboardService;
  readonly prompts: PromptStore;
  /** Opens a link the user asked for. The core checks it again before it does. */
  openLink(target: string): Promise<void>;
  /** Whether the key that opens the webview's developer tools is left to the webview. */
  readonly developerTools: boolean;
  /**
   * Offers a key to the shell before the program gets it. Returns whether the shell
   * took it: it ran a command, or the key begins a sequence that may.
   */
  shortcut(event: KeyboardEvent, context: { readonly alternateScreen: boolean }): boolean;
  /** Told the title the program gave its terminal, with nothing unsafe left in it. */
  title(title: string): void;
  /** Told that the view's terminal has taken the keyboard. */
  focused(): void;
  /** Starts a new session in place of one whose program has ended. */
  restart(): void;
  /**
   * Told that a search of the terminal has caught up with what the terminal now holds
   * and shows: there may be more for it to look through, and other numbers to show.
   */
  searched(): void;
}

/** Which way a search steps from the match it is at: to older text, or to newer. */
export type SearchDirection = "older" | "newer";

/** How far a search of a terminal has come. */
export interface SearchProgress {
  /**
   * What the search can do next. `idle`: nothing is being looked for. `held`: the
   * terminal has just been resized and the search waits for it to settle.
   */
  readonly status: SearchStatus | "idle" | "held";
  /** How many matches it has found so far. */
  readonly total: number;
  /** Which match it is at, counted from the newest, which is 0. */
  readonly selected: number | undefined;
  /** It has found as many as it counts and has stopped looking for more. */
  readonly capped: boolean;
}

/** The progress of a search that is not looking for anything. */
const NO_SEARCH: SearchProgress = { status: "idle", total: 0, selected: undefined, capped: false };

/** The visible screen of a view's terminal. Columns and rows count from zero. */
export interface ViewScreen {
  readonly session: string;
  readonly size: TermSize;
  readonly cursor: { readonly col: number; readonly row: number; readonly visible: boolean };
  /** Whether a full-screen program has switched to the alternate screen. */
  readonly alternate: boolean;
  /** One string for each row, without the blanks at its end. */
  readonly lines: readonly string[];
}

/** Where text that was waited for is on a view's screen. */
export interface ViewMatch {
  readonly session: string;
  readonly row: number;
  readonly line: string;
}

/** A pointer button held down, and what it is doing. */
interface Drag {
  readonly route: "report" | "select";
  readonly button: MouseButton | undefined;
  readonly pointerId: number;
  last: GridPoint;
  rectangle: boolean;
}

function sameSize(a: TermSize, b: TermSize): boolean {
  return a.cols === b.cols && a.rows === b.rows;
}

export class TerminalView implements FrameView {
  readonly #services: ViewServices;
  readonly #mapping: KeyMapping;
  readonly #mouse: MouseEncoder;
  readonly #wheel = new WheelAccumulator();

  #elements: ViewElements | undefined;
  #renderer: CellRenderer | undefined;
  #stopListening: (() => void)[] = [];

  #session: TerminalSession | undefined;
  /** The id of the session being shown. */
  #shown: string | undefined;
  #gesture: SelectionGesture | undefined;
  /** The search of the session's terminal, while there is something to look for. */
  #search: Search | undefined;
  /** What is being looked for. It outlives the session: a new one is searched too. */
  #needle = "";
  /** The runs the search last found in the viewport. One list, refilled. */
  readonly #highlights = newHighlights();
  /** What the terminal held and showed when the search was last brought up to date. */
  #searchStamp = "";
  /** The search must be brought up to date at the next frame whatever the stamp says. */
  #searchTouched = false;
  /** The newest match is to be selected as soon as there is one: a new search began. */
  #searchSelects = false;
  /** Until when the search waits for a resized terminal to settle, in page time. */
  #searchHeldUntil = 0;
  #searchHoldTimer: number | undefined;
  /**
   * A size the terminal was told to have whatever fits the view, and the grid that
   * fitted when it was told. The size holds until another grid fits: a caller's choice
   * is kept while the window stays as it is, and the window's user has the last word.
   */
  #pin: { readonly size: TermSize; readonly fitted: TermSize } | undefined;
  /** Run whenever the terminal's contents or size may have changed. */
  readonly #watchers = new Set<() => void>();
  /** Waiting for the view to have a size. */
  #measured: (() => void)[] = [];
  #programClipboard = new ProgramClipboard();
  /** The grid that last fitted the view's elements, kept for when it has none. */
  #lastFitted: TermSize | undefined;
  /** The grid the view is expected to have once it has elements. */
  #planned: TermSize | undefined;
  /** There is nothing to show and no session to wait for: the session has gone. */
  #lost = false;

  #appearance: Appearance;
  #cursor: CursorDefault;
  #scrollbackLines: number;
  #zoomPx: number;
  #dirty = false;
  #focused = false;
  #fontSizePx: number;
  #dpr = 1;
  #cssSize = { width: 0, height: 0 };
  #reportTimer: number | undefined;
  #lastFacts = "";

  #blinkTimer: number | undefined;
  #blinkOn = true;

  /** Keys whose press was sent, so their release is sent too and no other. */
  readonly #pressed = new Set<string>();
  #composing = false;
  /** Set for one task after a composition ends: some engines repeat its text as input. */
  #skipInput = false;

  #drag: Drag | undefined;
  #autoscrollTimer: number | undefined;
  /** Where the pointer last was, and whether the modifier that looks for links was held. */
  #pointer: { clientX: number; clientY: number; links: boolean } | undefined;
  #hover: LinkHit | undefined;
  /** Output arrived since the link under the pointer was found: it may have moved. */
  #hoverStale = false;
  /** A lasting message for the status line, such as that the program ended. */
  #message = "";
  #scrollbar = "";
  #inputAt = "";

  constructor(services: ViewServices, settings: ViewSettings) {
    this.#services = services;
    this.#appearance = settings.appearance;
    this.#cursor = settings.cursor;
    this.#scrollbackLines = settings.scrollbackLines;
    this.#zoomPx = settings.zoomPx;
    this.#fontSizePx = this.#fontSize();
    this.#mouse = new MouseEncoder(services.vt);
    this.#mapping = {
      platform: services.platform,
      macOptionAsAlt: MAC_OPTION_AS_ALT,
      layout: services.layout,
      knows: (code) => services.keys.knows(code),
    };
  }

  // Mounting.

  /** Starts working on `elements`. A view may be unmounted and mounted again. */
  mount(elements: ViewElements): void {
    if (this.#elements) this.unmount();
    this.#elements = elements;
    const { host } = this.#services;
    this.#styleElements();

    this.#renderer = host.createRenderer(
      elements.surface,
      rendererTheme(this.#appearance, this.#fontSizePx),
      {
        onInvalidate: () => this.#markDirty(),
        onTierChange: (tier, reason) => {
          if (tier === undefined)
            this.#setMessage(`The terminal can no longer be drawn: ${reason}`);
        },
      },
    );
    this.#renderer.setFocused(this.#focused);
    // A new renderer has none of the runs a search found.
    this.#searchTouched = true;

    const { root, input } = elements;
    this.#listen(input, "keydown", (event) => this.#onKeyDown(event));
    this.#listen(input, "keyup", (event) => this.#onKeyUp(event));
    this.#listen(input, "compositionstart", () => this.#onCompositionStart());
    this.#listen(input, "compositionupdate", (event) => this.#onCompositionUpdate(event));
    this.#listen(input, "compositionend", (event) => this.#onCompositionEnd(event));
    this.#listen(input, "input", (event) => this.#onInput(event as InputEvent));
    this.#listen(input, "paste", (event) => this.#onPasteEvent(event));
    this.#listen(input, "copy", (event) => this.#onCopyEvent(event));
    this.#listen(input, "cut", (event) => this.#onCopyEvent(event));
    this.#listen(input, "focus", () => {
      this.#setFocused(true);
      this.#services.focused();
    });
    this.#listen(input, "blur", () => this.#setFocused(false));
    this.#listen(root, "pointerdown", (event) => this.#onPointerDown(event));
    this.#listen(root, "pointermove", (event) => this.#onPointerMove(event));
    this.#listen(root, "pointerup", (event) => this.#onPointerUp(event));
    this.#listen(root, "pointercancel", (event) => this.#onPointerUp(event));
    this.#listen(root, "pointerleave", () => this.#onPointerLeave());
    this.#listen(root, "wheel", (event) => this.#onWheel(event), { passive: false });
    this.#listen(root, "contextmenu", (event) => event.preventDefault());
    // A session is told when its view goes out of sight and comes back.
    const visibility = () => this.#scheduleReport();
    document.addEventListener("visibilitychange", visibility);
    this.#stopListening.push(() => document.removeEventListener("visibilitychange", visibility));

    const observer = new ResizeObserver(() => this.#measure());
    observer.observe(root);
    this.#stopListening.push(() => observer.disconnect());
    this.#watchPixelRatio();

    this.#measure();
    host.loop.add(this);
    this.#showStatus();
    if (document.activeElement === input) this.#setFocused(true);
  }

  /** Stops working on the elements. The session, if any, is kept. */
  unmount(): void {
    if (!this.#elements) return;
    this.#services.host.loop.remove(this);
    for (const stop of this.#stopListening) stop();
    this.#stopListening = [];
    this.#stopBlink();
    this.#stopAutoscroll();
    if (this.#reportTimer !== undefined) window.clearTimeout(this.#reportTimer);
    this.#reportTimer = undefined;
    this.#drag = undefined;
    this.#renderer?.dispose();
    this.#renderer = undefined;
    this.#elements = undefined;
    this.#scrollbar = "";
    this.#inputAt = "";
    // A view that is not on show has neither the keyboard nor anyone looking at it,
    // and its session is told so: what it prints from here on is printed unwatched.
    this.#setFocused(false);
    if (this.#reportTimer !== undefined) window.clearTimeout(this.#reportTimer);
    this.#reportTimer = undefined;
    this.#report();
  }

  /** Unmounts, detaches from the session and frees what the view allocated. */
  dispose(): void {
    this.unmount();
    this.#dropSession();
    this.#mouse.free();
  }

  /** Gives the terminal the keyboard. */
  focus(): void {
    this.#elements?.input.focus({ preventScroll: true });
  }

  /** Whether the view has elements to work on. */
  get mounted(): boolean {
    return this.#elements !== undefined;
  }

  /** Whether the view's terminal has the keyboard. */
  get hasFocus(): boolean {
    return this.#focused;
  }

  // The look.

  /** The text's size in CSS pixels: the look's own, made larger or smaller by the zoom. */
  #fontSize(): number {
    const size = this.#appearance.fontSizePx + this.#zoomPx;
    return Math.min(Math.max(size, FONT_SIZE_MIN_PX), FONT_SIZE_MAX_PX);
  }

  /**
   * Gives the elements the look's colours and font. Styles are set through the style
   * object, which a policy that forbids inline style attributes still allows.
   */
  #styleElements(): void {
    const elements = this.#elements;
    if (!elements) return;
    const appearance = this.#appearance;
    elements.root.style.backgroundColor = cssColor(appearance.background);
    elements.preedit.style.color = cssColor(appearance.foreground);
    elements.preedit.style.backgroundColor = cssColor(appearance.background);
    elements.preedit.style.fontFamily = appearance.fontFamily;
    elements.input.style.fontFamily = appearance.fontFamily;
  }

  /**
   * Changes how the view is set up while it runs: a new theme, another font, a zoom. The
   * terminal's colours change at once, and a program that asked to hear of a change
   * between light and dark is told. The number of lines kept applies to the next
   * session.
   */
  configure(settings: ViewSettings): void {
    const before = this.#appearance;
    this.#appearance = settings.appearance;
    this.#scrollbackLines = settings.scrollbackLines;
    this.#zoomPx = settings.zoomPx;
    this.#fontSizePx = this.#fontSize();

    const session = this.#session;
    const cursorChanged =
      settings.cursor.style !== this.#cursor.style || settings.cursor.blink !== this.#cursor.blink;
    this.#cursor = settings.cursor;
    if (session) {
      const { terminal } = session;
      if (settings.appearance !== before) terminal.setColors(terminalColors(settings.appearance));
      if (settings.appearance.scheme !== before.scheme) {
        terminal.setColorScheme(settings.appearance.scheme);
        if (terminal.mode(Mode.ColorSchemeReport)) {
          session.send(encodeColorScheme(this.#services.vt, settings.appearance.scheme));
        }
      }
      if (cursorChanged) terminal.setDefaultCursor(settings.cursor.style, settings.cursor.blink);
    }

    this.#styleElements();
    this.#inputAt = "";
    this.#renderer?.setTheme(rendererTheme(settings.appearance, this.#fontSizePx));
    // Cells of another size make another grid, and the session hears of new colours.
    this.#measure();
    this.#scheduleReport();
    this.#markDirty();
  }

  // Size.

  /**
   * The grid the terminal should have: the one that fits the view at its current size,
   * unless a caller has asked for another.
   */
  gridSize(): TermSize {
    return this.#pin?.size ?? this.#fitted();
  }

  /**
   * The grid that fits the view at its current size. A view without elements, or one
   * that has not been laid out yet, answers with the grid it is expected to have, or
   * the one it last had.
   */
  #fitted(): TermSize {
    const renderer = this.#renderer;
    if (!renderer || this.#cssSize.width === 0) {
      return this.#planned ?? this.#lastFitted ?? { cols: 80, rows: 24 };
    }
    const grid = renderer.gridSize();
    this.#lastFitted = {
      cols: Math.min(Math.max(grid.cols, 1), MAX_DIMENSION),
      rows: Math.min(Math.max(grid.rows, 1), MAX_DIMENSION),
    };
    return this.#lastFitted;
  }

  /**
   * Says what grid the view will have once it has elements, for a view that has none:
   * a pane in a tab that is not on show still has a place in its tab. A session
   * attached to meanwhile is told this size.
   */
  setPlannedSize(size: TermSize): void {
    this.#planned = size;
  }

  /**
   * Resolves once the view has been laid out and knows what grid fits it. Before that
   * the grid is a guess, and a session started at a guess draws its first screen for
   * the wrong size.
   */
  whenMeasured(): Promise<void> {
    if (this.#renderer && this.#cssSize.width > 0) return Promise.resolve();
    return new Promise((resolve) => this.#measured.push(resolve));
  }

  /** The size of one cell in device pixels. */
  cellPx(): CellPx {
    // A view without a renderer measures the font the way its renderer will.
    const metrics =
      this.#renderer?.metrics ??
      this.#services.host.measureCells(
        rendererTheme(this.#appearance, this.#fontSizePx),
        window.devicePixelRatio,
      );
    return { w: metrics.cellWidth, h: metrics.cellHeight };
  }

  /** What a session is told about this view. */
  facts(): ClientFacts {
    const appearance = this.#appearance;
    return {
      size: this.gridSize(),
      cellPx: this.cellPx(),
      palette: factsPalette(appearance),
      scheme: appearance.scheme,
      focused: this.#focused,
      // A pane in a tab that is not on show is as hidden as a window that is.
      visible: this.#elements !== undefined && document.visibilityState === "visible",
    };
  }

  /** Reads the view's size again. The view does this itself when its element changes. */
  resize(): void {
    this.#measure();
  }

  #measure(): void {
    const elements = this.#elements;
    const renderer = this.#renderer;
    if (!elements || !renderer) return;
    const size = { width: elements.root.clientWidth, height: elements.root.clientHeight };
    if (size.width === 0 || size.height === 0) return;
    this.#cssSize = size;
    this.#dpr = window.devicePixelRatio;
    // The picture follows the element at once. The grid follows when the session has
    // answered, so until then the old grid is drawn in the new space.
    renderer.resize(size, this.#dpr);
    // Another grid fits now: the window or the font changed, and that is the newer wish.
    if (this.#pin && !sameSize(this.#fitted(), this.#pin.fitted)) this.#pin = undefined;
    this.#updateGeometry();
    this.#scheduleReport();
    this.#markDirty();
    for (const resolve of this.#measured.splice(0)) resolve();
  }

  /**
   * Notices a change of device pixel ratio, as when the window moves to another display.
   * A media query matches one ratio, so it is made again after each change.
   */
  #watchPixelRatio(): void {
    let stopped = false;
    let stop = () => undefined as void;
    const arm = () => {
      const query = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      const changed = () => {
        stop();
        if (stopped) return;
        this.#measure();
        arm();
      };
      query.addEventListener("change", changed);
      stop = () => query.removeEventListener("change", changed);
    };
    arm();
    this.#stopListening.push(() => {
      stopped = true;
      stop();
    });
  }

  /** Tells the encoders how the grid lies on the surface, in device pixels. */
  #updateGeometry(): void {
    const renderer = this.#renderer;
    const terminal = this.#session?.terminal;
    if (!renderer || !terminal) return;
    const m = renderer.metrics;
    const width = Math.round(this.#cssSize.width * this.#dpr);
    const height = Math.round(this.#cssSize.height * this.#dpr);
    this.#mouse.setGeometry({
      screenWidth: width,
      screenHeight: height,
      cellWidth: m.cellWidth,
      cellHeight: m.cellHeight,
      paddingTop: m.padding,
      paddingLeft: m.padding,
      paddingRight: Math.max(0, width - m.padding - terminal.cols * m.cellWidth),
      paddingBottom: Math.max(0, height - m.padding - terminal.rows * m.cellHeight),
    });
    this.#gesture?.setGeometry({
      columns: terminal.cols,
      cellWidth: m.cellWidth,
      paddingLeft: m.padding,
      screenHeight: height,
    });
    this.#gesture?.setClickTiming({
      repeatIntervalMs: CLICK_REPEAT_INTERVAL_MS,
      repeatDistancePx: CLICK_REPEAT_DISTANCE_CELLS * m.cellWidth,
    });
  }

  /** Tells the session the view's size and facts soon, once however often it is asked. */
  #scheduleReport(): void {
    if (this.#reportTimer !== undefined) return;
    this.#reportTimer = window.setTimeout(() => {
      this.#reportTimer = undefined;
      this.#report();
    }, REPORT_INTERVAL_MS);
  }

  #report(): void {
    const session = this.#session;
    if (!session) return;
    // Only a view that is laid out knows what size it is; the rest it always knows.
    if (this.#renderer) session.requestSize(this.gridSize(), this.cellPx());
    const facts = this.facts();
    const key = JSON.stringify(facts);
    if (key !== this.#lastFacts) {
      this.#lastFacts = key;
      session.facts(facts);
    }
  }

  // The session.

  /** The id of the session the view shows. */
  get session(): string | undefined {
    return this.#shown;
  }

  /**
   * Attaches the view to a session and starts showing it, in place of any it showed
   * before. With `size`, the terminal keeps that size instead of taking the view's.
   */
  async attach(
    client: TransportClient,
    backend: string,
    session: string,
    size?: TermSize,
  ): Promise<void> {
    const { vt } = this.#services;
    const appearance = this.#appearance;
    const pin = size ? { size, fitted: this.#fitted() } : undefined;
    const facts = { ...this.facts(), ...(size ? { size } : {}) };
    const attachment = await client.attach(backend, session, facts);
    this.#dropSession();
    this.#lost = false;
    this.#pin = pin;
    this.#shown = session;
    this.#lastFacts = JSON.stringify(facts);
    this.#programClipboard = new ProgramClipboard();
    const opened = new TerminalSession(vt, attachment, {
      colors: terminalColors(appearance),
      scheme: appearance.scheme,
      cellPx: facts.cellPx,
      scrollbackLines: this.#scrollbackLines,
      cursor: this.#cursor,
      hooks: {
        changed: () => {
          // What was under the pointer may have moved. It is looked for again when
          // the next frame has the screen as it now is.
          this.#hoverStale = this.#hover !== undefined;
          this.#markDirty();
          this.#notify();
        },
        resized: () => {
          this.#updateGeometry();
          this.#holdSearch();
          this.#notify();
        },
        // Nothing is said of it: what the program printed last is on the screen, and
        // that the shell is gone shows there too.
        exited: () => this.#markDirty(),
        title: (title) => this.#services.title(sanitizeTitle(title)),
        clipboardWrite: (text, location) => this.#onProgramClipboard(text, location),
      },
    });
    this.#session = opened;
    this.#gesture = new SelectionGesture(opened.terminal);
    // What was being looked for is looked for in the new session too.
    if (this.#needle !== "") this.#beginSearch();
    this.#setMessage("");
    this.#updateGeometry();
    opened.start();
    // The session has its own size; ask for this view's if it differs.
    this.#report();
    this.#markDirty();
  }

  /** Shows a lasting message on the status line: for failures the view cannot show itself. */
  showMessage(message: string): void {
    this.#setMessage(message);
  }

  /**
   * Says that the session the view was to show has gone and cannot be shown, or could
   * not be started: there is no screen to tell the user so. Enter starts a new one, as
   * it does in a view that saw its program end.
   */
  showLost(message: string): void {
    this.#dropSession();
    this.#lost = true;
    this.#setMessage(message);
  }

  /** Whether the view shows nothing that is running: its program ended, or went unseen. */
  get ended(): boolean {
    return this.#lost || this.#session?.exit !== undefined;
  }

  #dropSession(): void {
    this.#gesture?.free();
    this.#gesture = undefined;
    this.#endSearch();
    this.#session?.dispose();
    this.#session = undefined;
    this.#shown = undefined;
    this.#pin = undefined;
    this.#pressed.clear();
    // Whoever waits on the session that is gone learns so now.
    this.#notify();
  }

  // What a caller outside the window may ask of the terminal.

  /** The session the view shows, for a request that needs one. */
  #asked(): { readonly id: string; readonly open: TerminalSession } {
    const open = this.#session;
    const id = this.#shown;
    if (!open || id === undefined) {
      throw new CommandError("not_shown", "the pane is not showing a session");
    }
    return { id, open };
  }

  /** The visible screen of the session shown, as text. */
  screen(): ViewScreen {
    const { id, open } = this.#asked();
    const { terminal } = open;
    const cursor = terminal.cursor();
    return {
      session: id,
      size: { cols: terminal.cols, rows: terminal.rows },
      cursor: { col: cursor.x, row: cursor.y, visible: cursor.visible },
      alternate: terminal.alternateScreen(),
      lines: terminal.screenText(),
    };
  }

  /**
   * Resolves once `text` is on the screen of the session shown, with the first row that
   * holds it. Rejects with `timeout` after `timeoutMs`, and with `not_shown` if the view
   * turns to another session first.
   */
  waitText(text: string, timeoutMs: number): Promise<ViewMatch> {
    const { id, open } = this.#asked();
    return this.#until(
      open,
      () => {
        const lines = open.terminal.screenText();
        const row = lines.findIndex((line) => line.includes(text));
        return row < 0 ? undefined : { session: id, row, line: lines[row] ?? "" };
      },
      timeoutMs,
      "the text did not appear on the screen in time",
    );
  }

  /**
   * Gives the terminal of the session shown `size`, whatever fits the view, and resolves
   * once the terminal has it. The size holds until the view's own size next changes.
   */
  resizeTo(size: TermSize): Promise<TermSize> {
    const { open } = this.#asked();
    this.#pin = { size, fitted: this.#fitted() };
    this.#report();
    return this.#until(
      open,
      () => {
        const now = { cols: open.terminal.cols, rows: open.terminal.rows };
        return sameSize(now, size) ? now : undefined;
      },
      PINNED_SIZE_TIMEOUT_MS,
      "the terminal did not take the size in time",
    );
  }

  /** Resolves with what `probe` finds, looking again each time the terminal changes. */
  #until<T>(
    open: TerminalSession,
    probe: () => T | undefined,
    timeoutMs: number,
    late: string,
  ): Promise<T> {
    const found = probe();
    if (found !== undefined) return Promise.resolve(found);
    return new Promise((resolve, reject) => {
      const done = () => {
        this.#watchers.delete(watch);
        window.clearTimeout(timer);
      };
      const watch = () => {
        if (this.#session !== open) {
          done();
          reject(new CommandError("not_shown", "the pane turned to another session"));
          return;
        }
        const now = probe();
        if (now === undefined) return;
        done();
        resolve(now);
      };
      const timer = window.setTimeout(() => {
        done();
        reject(new CommandError("timeout", late));
      }, timeoutMs);
      this.#watchers.add(watch);
    });
  }

  #notify(): void {
    // Copied: a watcher that is done removes itself.
    for (const watch of [...this.#watchers]) watch();
  }

  // Drawing.

  needsFrame(): boolean {
    return this.#dirty && this.#renderer !== undefined && this.#session !== undefined;
  }

  frame(): void {
    const renderer = this.#renderer;
    const session = this.#session;
    if (!renderer || !session) return;
    const searched = this.#feedSearch(renderer, session);
    const frame = session.state.update();
    if (this.#hoverStale) {
      this.#hoverStale = false;
      this.#findHover();
    }
    // Cleared here, not before: finding the link again asks for a frame, which is this one.
    this.#dirty = false;
    this.#syncBlink(frame.cursor);
    renderer.draw(frame);
    session.state.markDrawn();
    this.#placeInput(frame.cursor);
    this.#updateScrollbar();
    if (searched) this.#services.searched();
  }

  // Searching.

  /**
   * Looks for `text` in the terminal from now on, scrollback included, in place of
   * whatever was being looked for. An empty text ends the search. The looking itself
   * is done by `searchPump`, a little at a time.
   */
  search(text: string): void {
    if (text === this.#needle) return;
    this.#needle = text;
    if (text === "") {
      this.#endSearch();
      this.#renderer?.setHighlights(undefined);
      this.#markDirty();
      return;
    }
    this.#beginSearch();
  }

  /**
   * Goes to the next match in a direction and brings it into view. After the oldest
   * comes the newest, and the other way about.
   */
  searchStep(direction: SearchDirection): void {
    const search = this.#search;
    if (!search) return;
    this.#searchSelects = false;
    if (!(direction === "older" ? search.next() : search.previous())) return;
    this.#revealMatch(search);
  }

  /**
   * Has the search look through more of the terminal for about `budgetMs` and says
   * where that left it. While the answer's status is `running` or `feed-required` and
   * it is not capped, there is more to look through.
   */
  searchPump(budgetMs: number): SearchProgress {
    const search = this.#search;
    if (!search) return NO_SEARCH;
    if (this.#searchHeld()) return { ...this.#progress(search), status: "held" };
    const before = search.total();
    search.pump(budgetMs, SEARCH_MATCHES_MAX);
    if (this.#searchSelects && search.total() > 0) {
      this.#searchSelects = false;
      if (search.next()) this.#revealMatch(search);
    } else if (search.total() !== before) {
      // More was found, some of it perhaps in the viewport.
      this.#searchTouched = true;
      this.#markDirty();
    }
    return this.#progress(search);
  }

  /** How far the search has come. */
  searchProgress(): SearchProgress {
    const search = this.#search;
    if (!search) return NO_SEARCH;
    const progress = this.#progress(search);
    return this.#searchHeld() ? { ...progress, status: "held" } : progress;
  }

  #progress(search: Search): SearchProgress {
    const total = search.total();
    return {
      status: search.status(),
      total,
      selected: search.selectedIndex(),
      capped: total >= SEARCH_MATCHES_MAX,
    };
  }

  /** Starts looking for the needle in the session's terminal, if there is a session. */
  #beginSearch(): void {
    const terminal = this.#session?.terminal;
    if (!terminal) return;
    this.#search ??= new Search(terminal);
    this.#search.setNeedle(this.#needle);
    this.#searchSelects = true;
    this.#searchTouched = true;
    this.#markDirty();
    // A view without elements draws no frame to say so from.
    this.#services.searched();
  }

  /** Frees the search. What was being looked for is kept. */
  #endSearch(): void {
    if (this.#searchHoldTimer !== undefined) window.clearTimeout(this.#searchHoldTimer);
    this.#searchHoldTimer = undefined;
    this.#searchHeldUntil = 0;
    this.#search?.free();
    this.#search = undefined;
    this.#searchStamp = "";
    this.#highlights.count = 0;
  }

  #searchHeld(): boolean {
    return this.#searchHeldUntil !== 0 && performance.now() < this.#searchHeldUntil;
  }

  /**
   * Has the search wait a moment: the terminal was resized. Its lines are laid out
   * anew, the search starts from nothing, and a window being dragged to a new size is
   * resized many times a second.
   */
  #holdSearch(): void {
    if (!this.#search) return;
    this.#searchHeldUntil = performance.now() + SEARCH_REFEED_INTERVAL_MS;
    // Where the matches were says nothing of where they are now.
    this.#highlights.count = 0;
    this.#renderer?.setHighlights(undefined);
    if (this.#searchHoldTimer !== undefined) window.clearTimeout(this.#searchHoldTimer);
    this.#searchHoldTimer = window.setTimeout(() => {
      this.#searchHoldTimer = undefined;
      this.#searchHeldUntil = 0;
      this.#searchTouched = true;
      this.#markDirty();
      this.#services.searched();
    }, SEARCH_REFEED_INTERVAL_MS);
  }

  /**
   * Brings the search up to date with the terminal before a frame is drawn, if the
   * terminal has changed or been scrolled since it last was, and hands the renderer
   * the matches now in the viewport. Returns whether it did.
   *
   * It is done here and nowhere else so that what is highlighted is never a frame
   * behind what is drawn, and a frame that changes nothing a search can see, such as
   * the cursor blinking, costs a search nothing.
   */
  #feedSearch(renderer: CellRenderer, session: TerminalSession): boolean {
    const search = this.#search;
    if (!search || this.#searchHeld()) return false;
    const { terminal } = session;
    const { total, offset } = terminal.scrollbar();
    const stamp = `${terminal.revision},${total},${offset}`;
    if (!this.#searchTouched && stamp === this.#searchStamp) return false;
    this.#searchTouched = false;
    this.#searchStamp = stamp;
    search.feed();
    search.readViewport(this.#highlights);
    renderer.setHighlights(this.#highlights);
    return true;
  }

  /**
   * Scrolls the match the search is at into view, to the middle of the pane, unless it
   * is in view already and clear of the search bar.
   */
  #revealMatch(search: Search): void {
    const terminal = this.#session?.terminal;
    const row = search.selectedRow();
    if (terminal && row !== undefined) {
      const { offset, length } = terminal.scrollbar();
      const clear = Math.min(SEARCH_CLEAR_ROWS, Math.max(0, length - 1));
      if (row < offset + clear || row >= offset + length) {
        terminal.scrollToRow(Math.max(0, row - Math.floor(length / 2)));
      }
    }
    this.#searchTouched = true;
    this.#markDirty();
  }

  #markDirty(): void {
    this.#dirty = true;
    this.#services.host.loop.requestFrame();
  }

  /** Blinks the cursor while the program asks for that and the terminal has the keyboard. */
  #syncBlink(cursor: FrameCursor): void {
    const blink =
      this.#focused && cursor.visible && cursor.blinking && this.#session?.exit === undefined;
    if (blink && this.#blinkTimer === undefined) {
      this.#blinkTimer = window.setInterval(() => {
        this.#blinkOn = !this.#blinkOn;
        this.#renderer?.setCursorVisible(this.#blinkOn);
        this.#markDirty();
      }, CURSOR_BLINK_INTERVAL_MS);
    } else if (!blink && this.#blinkTimer !== undefined) {
      this.#stopBlink();
    }
  }

  #stopBlink(): void {
    if (this.#blinkTimer !== undefined) window.clearInterval(this.#blinkTimer);
    this.#blinkTimer = undefined;
    this.#blinkOn = true;
    this.#renderer?.setCursorVisible(true);
  }

  /** Shows the cursor and starts its period again: it does not vanish while one types. */
  #pokeBlink(): void {
    if (this.#blinkTimer === undefined) return;
    this.#stopBlink();
  }

  /**
   * Keeps the input element on the cursor's cell. It is invisible, but an input method
   * opens its window of candidates where the element is.
   */
  #placeInput(cursor: FrameCursor): void {
    const elements = this.#elements;
    const renderer = this.#renderer;
    if (!elements || !renderer) return;
    const m = renderer.metrics;
    const left = (m.padding + cursor.x * m.cellWidth) / this.#dpr;
    const top = (m.padding + cursor.y * m.cellHeight) / this.#dpr;
    const height = m.cellHeight / this.#dpr;
    const at = `${left},${top},${height},${this.#fontSizePx}`;
    if (at === this.#inputAt) return;
    this.#inputAt = at;
    for (const element of [elements.input, elements.preedit]) {
      element.style.transform = `translate(${left}px, ${top}px)`;
      element.style.height = `${height}px`;
      element.style.lineHeight = `${height}px`;
      element.style.fontSize = `${this.#fontSizePx}px`;
    }
  }

  #updateScrollbar(): void {
    const thumb = this.#elements?.scrollThumb;
    const terminal = this.#session?.terminal;
    if (!thumb || !terminal) return;
    const { total, offset, length } = terminal.scrollbar();
    const key = `${total},${offset},${length}`;
    if (key === this.#scrollbar) return;
    this.#scrollbar = key;
    // With nothing scrolled off the screen there is nothing to show a position in.
    thumb.hidden = total <= length;
    if (thumb.hidden) return;
    thumb.style.top = `${(offset / total) * 100}%`;
    thumb.style.height = `${(length / total) * 100}%`;
  }

  // The status line.

  #setMessage(message: string): void {
    this.#message = message;
    this.#showStatus();
  }

  #showStatus(): void {
    const status = this.#elements?.status;
    if (!status) return;
    // Where a link leads is shown before it can be opened, and takes the line while
    // the pointer is on it.
    const text = this.#hover?.target ?? this.#message;
    status.textContent = text;
    status.hidden = text === "";
  }

  // The keyboard.

  #onKeyDown(event: KeyboardEvent): void {
    const terminal = this.#session?.terminal;
    const { platform, developerTools } = this.#services;
    if (systemKey(event, { platform, developerTools })) return;
    // The shell's own keys come first: a key that runs a command never reaches the
    // program, and neither does a key that may be the first of a sequence.
    const alternateScreen = terminal?.alternateScreen() ?? false;
    if (this.#services.shortcut(event, { alternateScreen })) {
      event.preventDefault();
      return;
    }
    if (unboundSystemKey(event)) return;

    // Nothing is left to type into. Enter, which would have been the next thing typed
    // at a shell, starts a new session instead.
    if (
      this.ended &&
      event.key === "Enter" &&
      !event.isComposing &&
      !(event.ctrlKey || event.altKey || event.metaKey || event.shiftKey)
    ) {
      event.preventDefault();
      this.#services.restart();
      return;
    }

    const input = mapKey(event, this.#mapping);
    // Not a key: text is on its way through the input method.
    if (input === null) return;
    if (isModifierKey(event)) {
      // Some programs ask to be told about modifiers alone. The key is not taken from
      // the webview, which needs it to make sense of the keys that follow.
      this.#sendKey(input, event.code, false);
      this.#refreshHover(event);
      return;
    }
    // Everything else is the terminal's. Left alone, keys such as F5 would act on the page.
    event.preventDefault();
    this.#sendKey(input, event.code, true);
  }

  #onKeyUp(event: KeyboardEvent): void {
    if (isModifierKey(event)) this.#refreshHover(event);
    const session = this.#session;
    if (!session || !this.#pressed.delete(event.code)) return;
    const bytes = this.#services.keys.encode(session.terminal, mapKeyRelease(event, this.#mapping));
    session.send(bytes);
  }

  #sendKey(input: KeyInput, code: string, typed: boolean): void {
    const session = this.#session;
    if (!session || session.exit !== undefined) return;
    const bytes = this.#services.keys.encode(session.terminal, input);
    if (bytes.length === 0) return;
    if (code !== "") this.#pressed.add(code);
    session.send(bytes);
    if (typed) this.#typed();
  }

  /** What follows anything the user sends: back to the live screen, selection gone. */
  #typed(): void {
    const terminal = this.#session?.terminal;
    if (!terminal) return;
    if (!terminal.viewportAtBottom()) terminal.scrollToBottom();
    if (hasSelection(terminal)) clearSelection(terminal);
    this.#pokeBlink();
    this.#markDirty();
  }

  #sendText(text: string): void {
    if (text !== "") this.#sendKey(textInput(text), "", true);
  }

  // Text that does not come from a key: input methods, dead keys, the emoji panel.

  #onCompositionStart(): void {
    this.#composing = true;
  }

  #onCompositionUpdate(event: CompositionEvent): void {
    const preedit = this.#elements?.preedit;
    if (!preedit) return;
    // Set as text: what is being composed is never markup.
    preedit.textContent = event.data;
    preedit.hidden = event.data === "";
  }

  #onCompositionEnd(event: CompositionEvent): void {
    this.#composing = false;
    const elements = this.#elements;
    if (elements) {
      elements.preedit.hidden = true;
      elements.preedit.textContent = "";
      elements.input.value = "";
    }
    // Some engines follow the end of a composition with an input event for the same
    // text. It arrives in this task or the next, and is not sent twice.
    this.#skipInput = true;
    window.setTimeout(() => (this.#skipInput = false), 0);
    this.#sendText(event.data);
  }

  #onInput(event: InputEvent): void {
    if (event.isComposing || this.#composing) return;
    const input = this.#elements?.input;
    // The element only passes text on; it never keeps any.
    if (input) input.value = "";
    if (this.#skipInput) return;
    this.#sendText(event.data ?? "");
  }

  // What a command may do to the terminal.

  /** Carries out one of the commands that act on a terminal. */
  perform(action: TerminalAction): void {
    const terminal = this.#session?.terminal;
    switch (action) {
      case "copy":
        void this.#copy();
        return;
      case "paste":
        void this.#paste();
        return;
    }
    if (!terminal) return;
    switch (action) {
      case "scroll-page-up":
        terminal.scrollBy(-Math.max(1, terminal.rows - 1));
        break;
      case "scroll-page-down":
        terminal.scrollBy(Math.max(1, terminal.rows - 1));
        break;
      case "scroll-top":
        terminal.scrollToTop();
        break;
      case "scroll-bottom":
        terminal.scrollToBottom();
        break;
      case "clear":
        // The history goes, and the program is asked to draw its screen afresh, which
        // is what a shell does with a form feed.
        clearSelection(terminal);
        terminal.clearScrollback();
        this.#session?.send(Uint8Array.of(0x0c));
        break;
      case "select-all":
        selectAll(terminal);
        break;
    }
    this.#markDirty();
  }

  // The clipboard.

  async #copy(): Promise<void> {
    const terminal = this.#session?.terminal;
    if (!terminal) return;
    const text = selectionText(terminal);
    if (text === "") return;
    try {
      await this.#services.clipboard.write(text);
    } catch (error) {
      this.#services.prompts.notice(`The selection could not be copied: ${describe(error)}`);
    }
  }

  async #paste(given?: string): Promise<void> {
    const { vt, clipboard, prompts } = this.#services;
    const session = this.#session;
    if (!session || session.exit !== undefined) return;
    let text: string | null;
    try {
      text = given ?? (await clipboard.read());
    } catch (error) {
      prompts.notice(`The clipboard could not be read: ${describe(error)}`);
      return;
    }
    if (text === null || text === "") return;

    const bracketed = session.terminal.mode(Mode.BracketedPaste);
    const decision = pasteDecision(text, isPasteSafe(vt, text), bracketed);
    if (decision === "too-large") {
      const limit = Math.round(PASTE_MAX_BYTES / (1024 * 1024));
      prompts.notice(`The clipboard holds more than can be pasted at once (${limit} MiB).`);
      return;
    }
    if (decision === "confirm") {
      const confirmed = await prompts.confirmPaste(pastePreview(text));
      this.focus();
      if (!confirmed) return;
    }
    // The answer may have taken a while: the session may be another one by now.
    if (this.#session !== session || session.exit !== undefined) return;
    session.send(encodePaste(session.terminal, text));
    this.#typed();
  }

  #onPasteEvent(event: ClipboardEvent): void {
    // A paste from a menu arrives here. It goes the same way as the shortcut's, with
    // the text the event carries.
    event.preventDefault();
    const text = event.clipboardData?.getData("text/plain") ?? "";
    if (text !== "") void this.#paste(text);
  }

  #onCopyEvent(event: ClipboardEvent): void {
    event.preventDefault();
    void this.#copy();
  }

  #onProgramClipboard(text: string, location: Parameters<ProgramClipboard["allows"]>[0]): void {
    const { clipboard, prompts } = this.#services;
    if (!this.#programClipboard.allows(location)) return;
    void clipboard.write(text).catch((error: unknown) => {
      console.error("a program's clipboard write failed", error);
    });
    if (this.#programClipboard.noticeDue()) {
      prompts.notice("The program in this terminal has put text on the clipboard.");
    }
  }

  // Focus.

  #setFocused(focused: boolean): void {
    if (focused === this.#focused) return;
    this.#focused = focused;
    this.#renderer?.setFocused(focused);
    const session = this.#session;
    if (session && session.terminal.mode(Mode.FocusEvent)) {
      session.send(encodeFocus(this.#services.vt, focused));
    }
    // A release that happens elsewhere is never seen, so no key is still held.
    if (!focused) this.#pressed.clear();
    this.#scheduleReport();
    this.#markDirty();
  }

  // The pointer.

  #point(event: { clientX: number; clientY: number }): GridPoint | undefined {
    const elements = this.#elements;
    const renderer = this.#renderer;
    const terminal = this.#session?.terminal;
    if (!elements || !renderer || !terminal) return undefined;
    const box = elements.root.getBoundingClientRect();
    const m = renderer.metrics;
    return gridPoint(
      (event.clientX - box.left) * this.#dpr,
      (event.clientY - box.top) * this.#dpr,
      {
        cols: terminal.cols,
        rows: terminal.rows,
        cellWidth: m.cellWidth,
        cellHeight: m.cellHeight,
        padding: m.padding,
      },
    );
  }

  #onPointerDown(event: PointerEvent): void {
    // The keyboard stays with the input element whatever is clicked.
    event.preventDefault();
    this.focus();
    const session = this.#session;
    const point = this.#point(event);
    if (!session || !point || this.#drag) return;
    const terminal = session.terminal;
    const button = mouseButton(event.button);
    const held = (route: Drag["route"]) => {
      this.#drag = {
        route,
        button,
        pointerId: event.pointerId,
        last: point,
        rectangle: event.altKey,
      };
      this.#elements?.root.setPointerCapture(event.pointerId);
    };

    if (pointerRoute(terminal.mouseTracking(), event.shiftKey) === "report") {
      if (!button) return;
      session.send(
        this.#mouse.encode(terminal, {
          action: "press",
          button,
          mods: pointerModifiers(event),
          x: point.x,
          y: point.y,
          anyButtonPressed: true,
        }),
      );
      held("report");
      return;
    }

    if (button !== "left") return;
    if (this.#linkModifier(event)) {
      const link = this.#linkAt(point);
      if (link) {
        void this.#openLink(link.target);
        return;
      }
    }
    if (event.shiftKey && extendSelection(terminal, point.col, point.row)) {
      this.#markDirty();
      return;
    }
    this.#gesture?.press(point, { timeMs: event.timeStamp });
    held("select");
    this.#markDirty();
  }

  #onPointerMove(event: PointerEvent): void {
    this.#pointer = {
      clientX: event.clientX,
      clientY: event.clientY,
      links: this.#linkModifier(event),
    };
    const session = this.#session;
    const point = this.#point(event);
    if (!session || !point) return;
    const terminal = session.terminal;
    const drag = this.#drag;

    if (drag?.route === "select") {
      drag.last = point;
      drag.rectangle = event.altKey;
      const direction = this.#gesture?.drag(point, { rectangle: event.altKey }) ?? "none";
      this.#autoscroll(direction);
      this.#markDirty();
      return;
    }
    if (
      drag?.route === "report" ||
      pointerRoute(terminal.mouseTracking(), event.shiftKey) === "report"
    ) {
      const motion = this.#mouse.encode(terminal, {
        action: "motion",
        mods: pointerModifiers(event),
        x: point.x,
        y: point.y,
        anyButtonPressed: event.buttons !== 0,
        ...(drag?.button ? { button: drag.button } : {}),
      });
      session.send(motion);
      return;
    }
    this.#findHover();
  }

  #onPointerUp(event: PointerEvent): void {
    const drag = this.#drag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    this.#drag = undefined;
    this.#stopAutoscroll();
    const session = this.#session;
    const point = this.#point(event) ?? drag.last;
    if (!session) return;
    if (drag.route === "select") {
      this.#gesture?.release(point);
      this.#markDirty();
      return;
    }
    if (!drag.button) return;
    session.send(
      this.#mouse.encode(session.terminal, {
        action: "release",
        button: drag.button,
        mods: pointerModifiers(event),
        x: point.x,
        y: point.y,
        anyButtonPressed: false,
      }),
    );
  }

  #onPointerLeave(): void {
    this.#pointer = undefined;
    this.#setHover(undefined);
  }

  /** Keeps the selection growing while the pointer is held past the edge of the grid. */
  #autoscroll(direction: Autoscroll): void {
    if (direction === "none") {
      this.#stopAutoscroll();
      return;
    }
    if (this.#autoscrollTimer !== undefined) return;
    this.#autoscrollTimer = window.setInterval(() => {
      const drag = this.#drag;
      const gesture = this.#gesture;
      if (!drag || drag.route !== "select" || !gesture) {
        this.#stopAutoscroll();
        return;
      }
      const still = gesture.autoscrollTick(drag.last, { rectangle: drag.rectangle });
      this.#markDirty();
      if (still === "none") this.#stopAutoscroll();
    }, AUTOSCROLL_INTERVAL_MS);
  }

  #stopAutoscroll(): void {
    if (this.#autoscrollTimer !== undefined) window.clearInterval(this.#autoscrollTimer);
    this.#autoscrollTimer = undefined;
  }

  #onWheel(event: WheelEvent): void {
    // The page itself never scrolls or zooms.
    event.preventDefault();
    const session = this.#session;
    const point = this.#point(event);
    if (!session || !point) return;
    const terminal = session.terminal;
    const lines = this.#wheel.lines(event, terminal.rows);
    if (lines === 0) return;
    const steps = Math.min(Math.abs(lines), terminal.rows);
    const route = wheelRoute(
      terminal.mouseTracking(),
      event.shiftKey,
      terminal.alternateScreen(),
      terminal.mode(Mode.MouseAlternateScroll),
    );
    if (route === "scroll") {
      terminal.scrollBy(lines);
      this.#markDirty();
      return;
    }
    for (let step = 0; step < steps; step++) {
      if (route === "report") {
        session.send(
          this.#mouse.encode(terminal, {
            action: "press",
            button: lines < 0 ? "four" : "five",
            mods: pointerModifiers(event),
            x: point.x,
            y: point.y,
          }),
        );
      } else {
        const code = lines < 0 ? "ArrowUp" : "ArrowDown";
        session.send(this.#services.keys.encode(terminal, { action: "press", code }));
      }
    }
  }

  // Links.

  /** Whether the modifier that turns the pointer towards links is held. */
  #linkModifier(event: { ctrlKey: boolean; metaKey: boolean }): boolean {
    return this.#services.platform === "mac" ? event.metaKey : event.ctrlKey;
  }

  #linkAt(point: GridPoint): LinkHit | undefined {
    const session = this.#session;
    if (!session) return undefined;
    const { terminal } = session;
    const frame = session.state.frame;
    const { col, row } = point;
    // The frame is the screen as last drawn; after a resize it may not be this grid yet.
    if (col >= frame.cols || row >= frame.rows || col >= terminal.cols || row >= terminal.rows) {
      return undefined;
    }
    const target = hyperlinkAt(terminal, col, row);
    if (target !== "") {
      const run = markedRun(frame, col, row, target, (c) => hyperlinkAt(terminal, c, row));
      return { target, ranges: [run] };
    }
    return addressUnder(frame, col, row) ?? undefined;
  }

  /** Looks again at what is under the pointer when a modifier is pressed or let go. */
  #refreshHover(event: { ctrlKey: boolean; metaKey: boolean }): void {
    if (!this.#pointer) return;
    this.#pointer.links = this.#linkModifier(event);
    this.#findHover();
  }

  /** Offers the link under the pointer, if the pointer is looking for links. */
  #findHover(): void {
    const pointer = this.#pointer;
    // A button that is down is selecting or belongs to the program.
    const point = pointer?.links && !this.#drag ? this.#point(pointer) : undefined;
    this.#setHover(point ? this.#linkAt(point) : undefined);
  }

  #setHover(hit: LinkHit | undefined): void {
    if (hit === undefined && this.#hover === undefined) return;
    // Only links of a kind that would be opened are offered.
    const link = hit && linkScheme(hit.target) ? hit : undefined;
    this.#hover = link;
    this.#renderer?.setUnderlines(link?.ranges ?? []);
    this.#elements?.root.classList.toggle("terminal-over-link", link !== undefined);
    this.#showStatus();
    this.#markDirty();
  }

  async #openLink(target: string): Promise<void> {
    const { prompts, openLink } = this.#services;
    const scheme = linkScheme(target);
    if (!scheme) return;
    if (LINK_SCHEMES_TO_CONFIRM.includes(scheme)) {
      const confirmed = await prompts.confirmLink(target);
      this.focus();
      if (!confirmed) return;
    }
    try {
      await openLink(target);
    } catch (error) {
      prompts.notice(`The link could not be opened: ${describe(error)}`);
    }
  }

  // Listening.

  #listen<K extends keyof HTMLElementEventMap>(
    target: HTMLElement,
    type: K,
    handler: (event: HTMLElementEventMap[K]) => void,
    options?: AddEventListenerOptions,
  ): void {
    const listener = handler as EventListener;
    target.addEventListener(type, listener, options);
    this.#stopListening.push(() => target.removeEventListener(type, listener, options));
  }
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
