/**
 * Selecting text with the pointer, and what is under a cell.
 *
 * The library turns pointer events into selections: it counts clicks, so a double click
 * selects a word and a triple click a line, it extends a selection as the pointer drags,
 * and it says when the pointer has left the grid and the view should scroll. It returns
 * the selection each event produces without applying it. Applying it to the terminal,
 * which makes it appear in the next frame, is done here.
 */

import { cellReader } from "./cell.ts";
import { NO_VALUE } from "./error.ts";
import type { Core } from "./module.ts";
import type { Terminal } from "./terminal.ts";

/** Where the pointer is. */
export interface PointerAt {
  /** The cell under the pointer, in viewport coordinates. */
  col: number;
  row: number;
  /** The pointer in pixels from the top-left corner of the surface. */
  x: number;
  y: number;
}

/** The surface the terminal is drawn on, as far as dragging is concerned. In pixels. */
export interface SelectionGeometry {
  columns: number;
  cellWidth: number;
  paddingLeft: number;
  screenHeight: number;
}

/**
 * When a press counts as a repeat of the one before it, making a double or triple click.
 * These follow the platform's double-click settings, which the caller knows.
 */
export interface ClickTiming {
  /** The longest time between two presses of one multi-click, in milliseconds. */
  repeatIntervalMs: number;
  /** The furthest apart two presses of one multi-click may be, in pixels. */
  repeatDistancePx: number;
}

/** Which way the view should scroll while a drag is held past its edge. */
export type Autoscroll = "none" | "up" | "down";

/** Bytes of scratch memory for one option value: the largest is two doubles. */
const VALUE_BYTES = 16;

/** Tracks one pointer's selection gesture on a terminal. */
export class SelectionGesture {
  readonly #terminal: Terminal;
  readonly #core: Core;
  #gesture: number;
  readonly #events: { press: number; release: number; drag: number; tick: number };
  readonly #option: Readonly<Record<string, number>>;
  readonly #scratch: number;
  readonly #scratchLen: number;
  readonly #selection: number;
  readonly #value: number;
  #geometry: SelectionGeometry | undefined;
  #timing: ClickTiming | undefined;

  constructor(terminal: Terminal) {
    this.#terminal = terminal;
    const core = terminal.core;
    this.#core = core;
    const m = core.manifest;
    this.#option = m.enumValues("GhosttySelectionGestureEventOption");

    const selectionSize = (m.struct("GhosttySelection").size + 7) & ~7;
    this.#scratchLen = selectionSize + VALUE_BYTES;
    this.#scratch = core.allocZeroed(this.#scratchLen);
    this.#selection = this.#scratch;
    this.#value = this.#scratch + selectionSize;

    this.#gesture = 0;
    this.#events = { press: 0, release: 0, drag: 0, tick: 0 };
    try {
      this.#gesture = core.newHandle("ghostty_selection_gesture_new", (out) =>
        core.x.ghostty_selection_gesture_new(0, out),
      );
      const event = (type: string) =>
        core.newHandle("ghostty_selection_gesture_event_new", (out) =>
          core.x.ghostty_selection_gesture_event_new(
            0,
            out,
            m.enumValue("GhosttySelectionGestureEventType", type),
          ),
        );
      this.#events.press = event("PRESS");
      this.#events.release = event("RELEASE");
      this.#events.drag = event("DRAG");
      this.#events.tick = event("AUTOSCROLL_TICK");
    } catch (err) {
      this.free();
      throw err;
    }
  }

  /** Describe the surface. Needed before a drag; call again when it changes. */
  setGeometry(geometry: SelectionGeometry): void {
    this.#geometry = { ...geometry };
  }

  /**
   * Say when presses count as a double or triple click. Until this is called every press
   * is a first click.
   */
  setClickTiming(timing: ClickTiming): void {
    this.#timing = { ...timing };
  }

  /**
   * The pointer was pressed. A press that repeats the one before it, by the click
   * timing, selects a word, and a third selects the line.
   *
   * `timeMs` is a monotonic time, such as a DOM event's `timeStamp`. Without it every
   * press counts as a first click.
   */
  press(at: PointerAt, options: { timeMs?: number } = {}): void {
    const event = this.#events.press;
    this.#setRef(event, at);
    this.#setPosition(event, at);
    if (options.timeMs === undefined) {
      this.#set(event, "TIME_NS", 0);
    } else {
      this.#core.view().setBigUint64(this.#value, BigInt(Math.round(options.timeMs * 1e6)), true);
      this.#set(event, "TIME_NS", this.#value);
    }
    if (this.#timing) {
      const view = this.#core.view();
      view.setBigUint64(this.#value, BigInt(Math.round(this.#timing.repeatIntervalMs * 1e6)), true);
      this.#set(event, "REPEAT_INTERVAL_NS", this.#value);
      view.setFloat64(this.#value, this.#timing.repeatDistancePx, true);
      this.#set(event, "REPEAT_DISTANCE", this.#value);
    }
    // A plain click produces no selection, which clears the one that was there.
    this.#apply(event, true);
  }

  /**
   * The pointer moved with the button held. Returns which way to scroll if it has left
   * the grid; while that is not `"none"`, call `autoscrollTick` from a timer.
   */
  drag(at: PointerAt, options: { rectangle?: boolean } = {}): Autoscroll {
    const event = this.#events.drag;
    this.#setRef(event, at);
    this.#setPosition(event, at);
    this.#setGeometry(event);
    this.#setBool(event, "RECTANGLE", options.rectangle ?? false);
    this.#apply(event, true);
    return this.autoscroll();
  }

  /** The button was released. `at` is left out when the pointer is not over a cell. */
  release(at?: PointerAt): void {
    const event = this.#events.release;
    if (at) this.#setRef(event, at);
    else this.#set(event, "REF", 0);
    this.#apply(event, false);
  }

  /**
   * Scroll one row in the direction the drag left the grid and extend the selection to
   * the pointer. Returns the direction that still applies; stop the timer on `"none"`.
   */
  autoscrollTick(at: PointerAt, options: { rectangle?: boolean } = {}): Autoscroll {
    const event = this.#events.tick;
    const m = this.#core.manifest;
    this.#setPosition(event, at);
    this.#setGeometry(event);
    this.#setBool(event, "RECTANGLE", options.rectangle ?? false);
    const view = this.#core.view();
    view.setUint16(this.#value + m.offset("GhosttyPointCoordinate", "x"), at.col, true);
    view.setUint32(this.#value + m.offset("GhosttyPointCoordinate", "y"), at.row, true);
    this.#set(event, "VIEWPORT", this.#value);
    // A tick that selects nothing leaves the current selection alone.
    this.#apply(event, false);
    return this.autoscroll();
  }

  /** Which way the view should be scrolling for the drag in progress. */
  autoscroll(): Autoscroll {
    const m = this.#core.manifest;
    const result = this.#core.x.ghostty_selection_gesture_get(
      this.#gesture,
      this.#terminal.handle,
      m.enumValue("GhosttySelectionGestureData", "AUTOSCROLL"),
      this.#value,
    );
    this.#core.check(result, "ghostty_selection_gesture_get");
    const direction = this.#core.view().getInt32(this.#value, true);
    const values = m.enumValues("GhosttySelectionGestureAutoscroll");
    return direction === values.UP ? "up" : direction === values.DOWN ? "down" : "none";
  }

  /** How many presses the gesture in progress has counted: 1, 2 or 3; 0 when idle. */
  clickCount(): number {
    const m = this.#core.manifest;
    const result = this.#core.x.ghostty_selection_gesture_get(
      this.#gesture,
      this.#terminal.handle,
      m.enumValue("GhosttySelectionGestureData", "CLICK_COUNT"),
      this.#value,
    );
    this.#core.check(result, "ghostty_selection_gesture_get");
    return this.#core.view().getUint8(this.#value);
  }

  /** Forget the gesture in progress. The terminal's selection is not touched. */
  reset(): void {
    this.#core.x.ghostty_selection_gesture_reset(this.#gesture, this.#terminal.handle);
  }

  /** Release the gesture. The object must not be used afterwards. */
  free(): void {
    const { x } = this.#core;
    for (const event of Object.values(this.#events)) {
      if (event !== 0) x.ghostty_selection_gesture_event_free(event);
    }
    this.#events.press = this.#events.release = this.#events.drag = this.#events.tick = 0;
    if (this.#gesture !== 0) {
      // The terminal may already be gone, which the library allows for.
      x.ghostty_selection_gesture_free(this.#gesture, this.#terminal.handle);
      this.#core.free(this.#scratch, this.#scratchLen);
    }
    this.#gesture = 0;
  }

  /** Run an event and apply the selection it produces. */
  #apply(event: number, clearWhenNone: boolean): void {
    const result = this.#core.x.ghostty_selection_gesture_event(
      this.#gesture,
      this.#terminal.handle,
      event,
      this.#selection,
    );
    if (result === NO_VALUE) {
      if (clearWhenNone) this.#terminal.setOption("SELECTION", 0);
      return;
    }
    this.#core.check(result, "ghostty_selection_gesture_event");
    this.#terminal.setOption("SELECTION", this.#selection);
  }

  #setRef(event: number, at: PointerAt): void {
    const ref = cellReader(this.#core).ref(this.#terminal.handle, "viewport", at.col, at.row);
    this.#set(event, "REF", ref);
  }

  #setPosition(event: number, at: PointerAt): void {
    const m = this.#core.manifest;
    const view = this.#core.view();
    view.setFloat64(this.#value + m.offset("GhosttySurfacePosition", "x"), at.x, true);
    view.setFloat64(this.#value + m.offset("GhosttySurfacePosition", "y"), at.y, true);
    this.#set(event, "POSITION", this.#value);
  }

  #setGeometry(event: number): void {
    const geometry = this.#geometry;
    if (!geometry) throw new Error("the selection gesture has no geometry yet");
    const m = this.#core.manifest;
    const view = this.#core.view();
    const set = (field: string, value: number) =>
      view.setUint32(
        this.#value + m.offset("GhosttySelectionGestureGeometry", field),
        Math.round(value),
        true,
      );
    set("columns", geometry.columns);
    set("cell_width", geometry.cellWidth);
    set("padding_left", geometry.paddingLeft);
    set("screen_height", geometry.screenHeight);
    this.#set(event, "GEOMETRY", this.#value);
  }

  #setBool(event: number, option: string, value: boolean): void {
    this.#core.view().setUint8(this.#value, value ? 1 : 0);
    this.#set(event, option, this.#value);
  }

  /** Set an option of an event. A `value` of 0 clears it. */
  #set(event: number, option: string, value: number): void {
    if (this.#gesture === 0) throw new Error("this selection gesture has been freed");
    const key = this.#option[option];
    if (key === undefined) throw new Error(`libghostty-vt has no gesture option ${option}`);
    this.#core.check(
      this.#core.x.ghostty_selection_gesture_event_set(event, key, value),
      `ghostty_selection_gesture_event_set(${option})`,
    );
  }
}

/** Select everything, scrollback included. Does nothing on an empty terminal. */
export function selectAll(terminal: Terminal): void {
  const core = terminal.core;
  const selection = core.scratch(core.manifest.struct("GhosttySelection").size);
  const result = core.x.ghostty_terminal_select_all(terminal.handle, selection);
  if (result === NO_VALUE) return;
  core.check(result, "ghostty_terminal_select_all");
  terminal.setOption("SELECTION", selection);
}

/** Remove the selection. */
export function clearSelection(terminal: Terminal): void {
  terminal.setOption("SELECTION", 0);
}

/** Whether anything is selected. */
export function hasSelection(terminal: Terminal): boolean {
  const core = terminal.core;
  return terminal.getData("SELECTION", core.scratch(core.manifest.struct("GhosttySelection").size));
}

/**
 * The target of the hyperlink on a cell of the viewport, or `""` when the cell is not
 * part of one. The target is whatever the program sent: validate it before opening it.
 */
export function hyperlinkAt(terminal: Terminal, col: number, row: number): string {
  const reader = cellReader(terminal.core);
  reader.ref(terminal.handle, "viewport", col, row);
  return reader.hyperlinkUri();
}
