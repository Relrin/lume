/**
 * How large a pane that does not exist yet is going to be.
 *
 * A session is started before its pane is made, and a session started at the size its
 * pane turns out to have draws its first screen once instead of twice. So the page
 * works out where an intent would put its new pane, by doing to a copy of the tab's
 * tree what the core will do to the tree itself, and from that rectangle the grid.
 */

import type { CellMetrics } from "@lume/renderer";

import { MAX_DIMENSION, type TermSize } from "../transport/types.ts";
import { computeRects, unitRectToPx, type Rect, type ShapeNode } from "./geometry.ts";
import type { LayoutState, TreeNode } from "./store.ts";
import {
  FLOAT_CASCADE,
  FLOAT_DEFAULT,
  FLOAT_MIN,
  SHARE_MIN,
  UNITS,
  type Axis,
  type Dir,
  type Intent,
  type PaneId,
  type UnitRect,
} from "./types.ts";

/** Stands for the pane an intent would make. No pane of a layout is called this. */
const NEW_PANE: PaneId = "(new)";

/** Stands for the split an intent would make. */
const NEW_SPLIT = "(new)";

/** How many floating panes are placed one beside the other before the places repeat. */
const FLOAT_CASCADE_STEPS = 8;

/** The parts of a card that are not terminal, in CSS pixels. */
export interface CardChrome {
  /** The height of the card's header; nought when it has none. */
  readonly headerPx: number;
  /** The width of the card's border on each side. */
  readonly borderPx: number;
}

/** A tab's tree with each split's ratios, as the store has them. */
export function shapeOf(state: LayoutState, node: TreeNode): ShapeNode {
  if (node.type === "pane") return node;
  return {
    type: "split",
    id: node.id,
    axis: node.axis,
    ratios: state.splits[node.id]?.ratios ?? [],
    children: node.children.map((child) => shapeOf(state, child)),
  };
}

/** The place of the largest of some numbers: the last, of several alike. */
function largest(values: readonly number[]): number {
  let at = 0;
  values.forEach((value, index) => {
    if (value >= (values[at] ?? 0)) at = index;
  });
  return at;
}

/**
 * The shares of a split's children once the split itself has only `share` of its
 * parent: what rounding loses goes to the largest.
 */
function within(share: number, ratios: readonly number[]): number[] {
  const scaled = ratios.map((ratio) => Math.floor((share * ratio) / UNITS));
  const sum = scaled.reduce((total, value) => total + value, 0);
  if (scaled.length > 0) scaled[largest(scaled)]! += Math.max(0, share - sum);
  return scaled;
}

/** A tree in its one proper form: a row directly inside a row is one row. */
function tidy(node: ShapeNode): ShapeNode {
  if (node.type === "pane") return node;
  const ratios: number[] = [];
  const children: ShapeNode[] = [];
  node.children.forEach((child, index) => {
    const share = node.ratios?.[index] ?? 0;
    const tidied = tidy(child);
    if (tidied.type === "split" && tidied.axis === node.axis) {
      ratios.push(...within(share, tidied.ratios ?? []));
      children.push(...tidied.children);
    } else {
      ratios.push(share);
      children.push(tidied);
    }
  });
  const sum = ratios.reduce((total, value) => total + value, 0);
  if (ratios.length > 0) ratios[largest(ratios)]! += UNITS - sum;
  return { type: "split", id: node.id, axis: node.axis, ratios, children };
}

/** The tree with a new pane on one side of `target`, taking `ratio` of its space. */
function withSplit(node: ShapeNode, target: PaneId, dir: Dir, ratio: number): ShapeNode {
  if (node.type === "split") {
    return {
      ...node,
      children: node.children.map((child) => withSplit(child, target, dir, ratio)),
    };
  }
  if (node.id !== target) return node;
  const axis: Axis = dir === "left" || dir === "right" ? "row" : "column";
  const made: ShapeNode = { type: "pane", id: NEW_PANE };
  const first = dir === "left" || dir === "up";
  return {
    type: "split",
    id: NEW_SPLIT,
    axis,
    ratios: first ? [ratio, UNITS - ratio] : [UNITS - ratio, ratio],
    children: first ? [made, node] : [node, made],
  };
}

/** Where a pane floats when nothing says where, over a window with `floating` panes. */
function defaultFloat(floating: number): UnitRect {
  const origin = (UNITS - FLOAT_DEFAULT) / 2;
  const step = FLOAT_CASCADE * (floating % FLOAT_CASCADE_STEPS);
  const side = Math.min(Math.max(FLOAT_DEFAULT, FLOAT_MIN), UNITS);
  return {
    x: Math.min(origin + step, UNITS - side),
    y: Math.min(origin + step, UNITS - side),
    w: side,
    h: side,
  };
}

/**
 * The rectangle of the pane an intent would make, or nothing for an intent that makes
 * none or that the layout would refuse. `box` is the space a window's tabs are laid
 * out in, and floating panes float over; `gapPx` is the gap between neighbouring panes.
 */
export function estimatePane(
  state: LayoutState,
  intent: Intent,
  box: Rect,
  gapPx: number,
): Rect | undefined {
  switch (intent.type) {
    case "split": {
      const tab = state.tabs[state.paneTabs[intent.pane] ?? ""];
      if (!tab) return undefined;
      if (!(intent.ratio >= SHARE_MIN && intent.ratio <= UNITS - SHARE_MIN)) return undefined;
      // Dividing a pane ends any zoom, so the tab is laid out whole.
      const after = tidy(
        withSplit(shapeOf(state, tab.root), intent.pane, intent.dir, intent.ratio),
      );
      return computeRects(after, box, gapPx).panes.find((placed) => placed.pane === NEW_PANE)?.rect;
    }
    case "tab_new":
    case "window_new":
      return computeRects({ type: "pane", id: NEW_PANE }, box, gapPx).panes[0]?.rect;
    case "scratch_show": {
      // The scratch terminal is made once; after that it is only shown again.
      if (state.scratch !== null || !intent.content) return undefined;
      const window = state.windows[intent.window];
      if (!window) return undefined;
      return unitRectToPx(defaultFloat(window.floating.length), box);
    }
    default:
      return undefined;
  }
}

/**
 * The grid of a terminal in a card of this size: the card less its header and
 * borders, in device pixels, divided into whole cells with the terminal's padding
 * taken off both ways. At least one cell each way, and no more than a terminal may
 * have.
 */
export function gridFor(
  card: Rect,
  chrome: CardChrome,
  metrics: CellMetrics,
  dpr: number,
): TermSize {
  const width = Math.max(0, Math.round((card.width - 2 * chrome.borderPx) * dpr));
  const height = Math.max(
    0,
    Math.round((card.height - chrome.headerPx - 2 * chrome.borderPx) * dpr),
  );
  const fit = (extent: number, cell: number) => {
    const cells = Math.floor((extent - 2 * metrics.padding) / Math.max(1, cell));
    return Math.min(Math.max(cells, 1), MAX_DIMENSION);
  };
  return { cols: fit(width, metrics.cellWidth), rows: fit(height, metrics.cellHeight) };
}
