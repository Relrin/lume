/**
 * Where a floating pane goes while the pointer moves or resizes it.
 *
 * A floating pane is kept inside the space it floats over and is never made smaller
 * than a terminal is of use at. Nothing here touches the page: the same function turns
 * every pointer position of a drag into the rectangle the card is drawn at.
 */

import type { Rect } from "./geometry.ts";

/**
 * What of a floating pane the pointer has hold of: the whole of it by its header, or
 * one of its edges or lower corners.
 */
export type FloatGrip = "move" | "w" | "e" | "s" | "sw" | "se";

/** The grips that resize a floating pane, in the order their handles are rendered. */
export const FLOAT_EDGES = ["w", "e", "s", "sw", "se"] as const satisfies readonly FloatGrip[];

/** A width and a height in CSS pixels. */
export interface Extent {
  readonly width: number;
  readonly height: number;
}

function clamp(value: number, least: number, most: number): number {
  return Math.min(Math.max(value, least), Math.max(least, most));
}

/** A rectangle moved and, only if it is larger than the box, shrunk to lie inside it. */
export function keepInside(rect: Rect, box: Rect): Rect {
  const width = Math.min(rect.width, box.width);
  const height = Math.min(rect.height, box.height);
  return {
    x: clamp(rect.x, box.x, box.x + box.width - width),
    y: clamp(rect.y, box.y, box.y + box.height - height),
    width,
    height,
  };
}

/**
 * The rectangle of a floating pane that started a drag at `start`, once the pointer
 * has moved by `dx` and `dy`. Moving keeps the size; an edge moves alone and stops at
 * the box's edge and where the pane would become smaller than `min`. A box smaller
 * than `min` allows what it has.
 */
export function dragFloat(
  start: Rect,
  grip: FloatGrip,
  dx: number,
  dy: number,
  box: Rect,
  min: Extent,
): Rect {
  if (grip === "move") {
    return keepInside({ ...start, x: start.x + dx, y: start.y + dy }, box);
  }
  const least = {
    width: Math.min(min.width, box.width),
    height: Math.min(min.height, box.height),
  };
  let { x, width, height } = start;
  const right = start.x + start.width;
  if (grip === "e" || grip === "se") {
    width = clamp(start.width + dx, least.width, box.x + box.width - start.x);
  }
  if (grip === "w" || grip === "sw") {
    x = clamp(start.x + dx, box.x, right - least.width);
    width = right - x;
  }
  if (grip === "s" || grip === "sw" || grip === "se") {
    height = clamp(start.height + dy, least.height, box.y + box.height - start.y);
  }
  return { x, y: start.y, width, height };
}
