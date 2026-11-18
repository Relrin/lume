/**
 * Deciding what the pointer and the wheel are for.
 *
 * A program can ask to be told about the mouse, and then clicks and the wheel are its
 * own. Holding Shift always gives them back to the terminal, so text can be selected and
 * the scrollback reached even inside a program that takes the mouse.
 */

import { Mod, type MouseButton } from "@lume/vt";

import { WHEEL_PIXELS_PER_LINE } from "../tuning.ts";

/** Who a pointer press, move or release goes to. */
export type PointerRoute = "report" | "select";

/** Who the pointer belongs to: the program, if it tracks the mouse and Shift is not held. */
export function pointerRoute(tracking: boolean, shift: boolean): PointerRoute {
  return tracking && !shift ? "report" : "select";
}

/** What turning the wheel does. */
export type WheelRoute = "report" | "arrows" | "scroll";

/**
 * What the wheel does. A program that tracks the mouse gets wheel events. A full-screen
 * program that does not, such as a pager, gets arrow keys if it asked for them, because
 * it has no scrollback of the terminal's to move through. Otherwise the wheel scrolls
 * the terminal's own scrollback.
 */
export function wheelRoute(
  tracking: boolean,
  shift: boolean,
  alternateScreen: boolean,
  alternateScroll: boolean,
): WheelRoute {
  if (tracking && !shift) return "report";
  if (alternateScreen && alternateScroll && !shift) return "arrows";
  return "scroll";
}

/** The parts of a DOM `WheelEvent` the accumulator reads. */
export interface WheelLike {
  readonly deltaY: number;
  /** 0 for pixels, 1 for lines, 2 for pages, as the DOM numbers them. */
  readonly deltaMode: number;
}

/**
 * Turns wheel movement into whole lines.
 *
 * A touchpad reports many small movements, so the part of a line that has not been used
 * yet is kept for the next event instead of being lost or rounded up.
 */
export class WheelAccumulator {
  #pending = 0;

  /**
   * The lines to move for an event, positive towards newer output. `rows` is the height
   * of the view, which is what a page is.
   */
  lines(event: WheelLike, rows: number): number {
    const moved =
      event.deltaMode === 1
        ? event.deltaY
        : event.deltaMode === 2
          ? event.deltaY * Math.max(1, rows - 1)
          : event.deltaY / WHEEL_PIXELS_PER_LINE;
    // A change of direction starts afresh rather than first undoing what was left over.
    if (Math.sign(moved) !== Math.sign(this.#pending)) this.#pending = 0;
    this.#pending += moved;
    const whole = Math.trunc(this.#pending);
    this.#pending -= whole;
    // Less than a line upwards truncates to a negative zero; callers get a plain one.
    return whole || 0;
  }

  reset(): void {
    this.#pending = 0;
  }
}

/** The engine's name for a DOM mouse button, if a program can be told about it. */
export function mouseButton(button: number): MouseButton | undefined {
  switch (button) {
    case 0:
      return "left";
    case 1:
      return "middle";
    case 2:
      return "right";
    // The back and forward buttons, by the numbers terminals have always given them.
    case 3:
      return "eight";
    case 4:
      return "nine";
    default:
      return undefined;
  }
}

/** The modifier bits a pointer or wheel event carries. */
export function pointerModifiers(event: {
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}): number {
  let mods = 0;
  if (event.shiftKey) mods |= Mod.Shift;
  if (event.ctrlKey) mods |= Mod.Ctrl;
  if (event.altKey) mods |= Mod.Alt;
  if (event.metaKey) mods |= Mod.Super;
  return mods;
}

/** Where the pointer is on the grid. */
export interface GridPoint {
  /** The cell under the pointer, held inside the grid. */
  col: number;
  row: number;
  /** The pointer in device pixels from the top-left corner of the surface. */
  x: number;
  y: number;
}

/** The grid a pointer position is measured against, in device pixels. */
export interface GridGeometry {
  cols: number;
  rows: number;
  cellWidth: number;
  cellHeight: number;
  /** From the left and top edges of the surface to the grid. */
  originX: number;
  originY: number;
}

/** The grid point for a position in device pixels. */
export function gridPoint(x: number, y: number, grid: GridGeometry): GridPoint {
  const clamp = (value: number, most: number) => Math.min(Math.max(value, 0), most);
  return {
    col: clamp(Math.floor((x - grid.originX) / grid.cellWidth), grid.cols - 1),
    row: clamp(Math.floor((y - grid.originY) / grid.cellHeight), grid.rows - 1),
    x,
    y,
  };
}
