/**
 * Where a card that is being dragged would go if it were let go now.
 *
 * A card dragged over another card of its tab can be dropped at one of that card's
 * edges, to sit beside it on that side, or in its middle, to change places with it.
 * Nothing here touches the page: the function takes a point and the cards' rectangles
 * and names the target, with the rectangle a preview of it is drawn at.
 */

import type { PaneRect, Rect } from "../layout/geometry.ts";
import type { Dir, PaneId } from "../layout/types.ts";
import { DROP_EDGE_FRACTION } from "../tuning.ts";

/** A point in the same coordinates as the cards' rectangles. */
export interface Point {
  readonly x: number;
  readonly y: number;
}

/** What letting a dragged card go would do. */
export type DropTarget =
  /** Put the card on one side of another, which gives up half its space. */
  | { readonly kind: "beside"; readonly pane: PaneId; readonly edge: Dir; readonly rect: Rect }
  /** Exchange the card's place with another's. */
  | { readonly kind: "swap"; readonly pane: PaneId; readonly rect: Rect };

function inside(point: Point, rect: Rect): boolean {
  return (
    point.x >= rect.x &&
    point.x < rect.x + rect.width &&
    point.y >= rect.y &&
    point.y < rect.y + rect.height
  );
}

/** The half of a card on one of its sides. */
function half(rect: Rect, edge: Dir): Rect {
  const width = Math.round(rect.width / 2);
  const height = Math.round(rect.height / 2);
  switch (edge) {
    case "left":
      return { ...rect, width };
    case "right":
      return { ...rect, x: rect.x + rect.width - width, width };
    case "up":
      return { ...rect, height };
    case "down":
      return { ...rect, y: rect.y + rect.height - height, height };
  }
}

/**
 * The target under `point` for the card of `dragged`, or nothing when the point is
 * over no other card. A point within `edgeFraction` of a card's extent from one of its
 * edges means that edge, the nearest one where two are that near; anywhere else on the
 * card means changing places with it.
 */
export function dropTarget(
  point: Point,
  panes: readonly PaneRect[],
  dragged: PaneId,
  edgeFraction: number = DROP_EDGE_FRACTION,
): DropTarget | undefined {
  const over = panes.find((placed) => placed.pane !== dragged && inside(point, placed.rect));
  if (!over || over.rect.width <= 0 || over.rect.height <= 0) return undefined;
  const { rect, pane } = over;
  const across = (point.x - rect.x) / rect.width;
  const down = (point.y - rect.y) / rect.height;
  // How far the point is from each edge, as a share of the card's extent that way.
  const distances: readonly (readonly [Dir, number])[] = [
    ["left", across],
    ["right", 1 - across],
    ["up", down],
    ["down", 1 - down],
  ];
  let nearest: readonly [Dir, number] | undefined;
  for (const candidate of distances) {
    if (candidate[1] >= edgeFraction) continue;
    if (!nearest || candidate[1] < nearest[1]) nearest = candidate;
  }
  if (!nearest) return { kind: "swap", pane, rect };
  return { kind: "beside", pane, edge: nearest[0], rect: half(rect, nearest[0]) };
}
