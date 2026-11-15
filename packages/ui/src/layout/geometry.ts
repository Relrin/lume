/**
 * Where the panes of a tab lie in a window, in pixels.
 *
 * The layout holds ratios; a page turns them into rectangles for the space it has.
 * Everything here is whole pixels: the children of a split are separated by exactly
 * the gap and fill the split's extent exactly, the last child taking what rounding
 * left over, so cards never overlap and never leave a sliver between them.
 *
 * Nothing here touches the page. The same functions lay out what is on screen, turn a
 * dragged divider into ratios, and say how large a pane that does not exist yet
 * would be.
 */

import { DIVIDER_HIT_MIN_PX } from "../tuning.ts";
import { SHARE_MIN, UNITS, type Axis, type PaneId, type SplitId, type UnitRect } from "./types.ts";

/** A rectangle in CSS pixels. */
export interface Rect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/**
 * A tree to lay out: a tab's tree as the core writes it, or only its shape with the
 * ratios looked up elsewhere.
 */
export type ShapeNode =
  | { readonly type: "pane"; readonly id: PaneId }
  | {
      readonly type: "split";
      readonly id: SplitId;
      readonly axis: Axis;
      readonly children: readonly ShapeNode[];
      /** Each child's share, in units. May be left out when `ratios` is given to the layout. */
      readonly ratios?: readonly number[];
    };

/** Where one pane lies. */
export interface PaneRect {
  readonly pane: PaneId;
  readonly rect: Rect;
}

/** The gap between two neighbouring children of a split, which a pointer may drag. */
export interface Divider {
  readonly split: SplitId;
  /** The divider lies between the split's children `index` and `index + 1`. */
  readonly index: number;
  /** The split's direction: a divider of a row is dragged sideways. */
  readonly axis: Axis;
  /** What a pointer can take hold of: the gap, widened evenly when it is very narrow. */
  readonly rect: Rect;
  /** The gap itself. */
  readonly gap: Rect;
  /** The extent of the child before the divider along the split's direction. */
  readonly beforePx: number;
  /** The extent of the child after it. */
  readonly afterPx: number;
  /** The split's whole extent along its direction, gaps included. */
  readonly extentPx: number;
}

/** A tab laid out. */
export interface Rects {
  /** The panes in the order they are laid out. */
  readonly panes: readonly PaneRect[];
  readonly dividers: readonly Divider[];
}

/** How a tree is laid out, beyond its own contents. */
export interface LayoutOptions {
  /**
   * A split's ratios, in place of those the tree carries: the store's, or the ones a
   * drag is trying out. A split it knows nothing of keeps the tree's own.
   */
  ratios?(split: SplitId): readonly number[] | undefined;
  /** The pane that fills the box alone, hiding the others. */
  zoomed?: PaneId | null;
}

function round(rect: Rect): Rect {
  const x = Math.round(rect.x);
  const y = Math.round(rect.y);
  return {
    x,
    y,
    width: Math.max(0, Math.round(rect.x + rect.width) - x),
    height: Math.max(0, Math.round(rect.y + rect.height) - y),
  };
}

function contains(node: ShapeNode, pane: PaneId): boolean {
  if (node.type === "pane") return node.id === pane;
  return node.children.some((child) => contains(child, pane));
}

/**
 * The extents of a split's children along its direction: whole pixels that add up to
 * `available` exactly.
 */
function shares(ratios: readonly number[], count: number, available: number): number[] {
  const sizes: number[] = [];
  let used = 0;
  for (let index = 0; index < count; index++) {
    // A split whose ratios are missing or too few shares alike.
    const ratio = ratios.length === count ? (ratios[index] ?? 0) : UNITS / count;
    const size = index === count - 1 ? available - used : Math.floor((available * ratio) / UNITS);
    const fitted = Math.max(0, Math.min(size, available - used));
    sizes.push(fitted);
    used += fitted;
  }
  return sizes;
}

function place(
  node: ShapeNode,
  rect: Rect,
  gapPx: number,
  options: LayoutOptions,
  panes: PaneRect[],
  dividers: Divider[],
): void {
  if (node.type === "pane") {
    panes.push({ pane: node.id, rect });
    return;
  }
  const count = node.children.length;
  if (count === 0) return;
  const row = node.axis === "row";
  const start = row ? rect.x : rect.y;
  const extent = row ? rect.width : rect.height;
  const available = Math.max(0, extent - gapPx * (count - 1));
  const ratios = options.ratios?.(node.id) ?? node.ratios ?? [];
  const sizes = shares(ratios, count, available);

  let cursor = start;
  node.children.forEach((child, index) => {
    const size = sizes[index] ?? 0;
    const childRect: Rect = row
      ? { x: cursor, y: rect.y, width: size, height: rect.height }
      : { x: rect.x, y: cursor, width: rect.width, height: size };
    place(child, childRect, gapPx, options, panes, dividers);
    cursor += size;
    if (index === count - 1) return;

    const widen = Math.ceil(Math.max(0, DIVIDER_HIT_MIN_PX - gapPx) / 2);
    const gap: Rect = row
      ? { x: cursor, y: rect.y, width: gapPx, height: rect.height }
      : { x: rect.x, y: cursor, width: rect.width, height: gapPx };
    const hit: Rect = row
      ? { x: cursor - widen, y: rect.y, width: gapPx + 2 * widen, height: rect.height }
      : { x: rect.x, y: cursor - widen, width: rect.width, height: gapPx + 2 * widen };
    dividers.push({
      split: node.id,
      index,
      axis: node.axis,
      rect: hit,
      gap,
      beforePx: size,
      afterPx: sizes[index + 1] ?? 0,
      extentPx: extent,
    });
    cursor += gapPx;
  });
}

/**
 * Lays a tab's tree out in `box`, with `gapPx` between neighbouring panes. The box is
 * taken to whole pixels first. With a zoomed pane, that pane fills the box and there
 * are no dividers.
 */
export function computeRects(
  root: ShapeNode,
  box: Rect,
  gapPx: number,
  options: LayoutOptions = {},
): Rects {
  const whole = round(box);
  const { zoomed } = options;
  if (zoomed !== undefined && zoomed !== null && contains(root, zoomed)) {
    return { panes: [{ pane: zoomed, rect: whole }], dividers: [] };
  }
  const panes: PaneRect[] = [];
  const dividers: Divider[] = [];
  place(root, whole, Math.max(0, Math.round(gapPx)), options, panes, dividers);
  return { panes, dividers };
}

/** A rectangle given in units of a box, in the box's pixels. */
export function unitRectToPx(rect: UnitRect, box: Rect): Rect {
  const across = (value: number, extent: number) => Math.round((value * extent) / UNITS);
  const x = box.x + across(rect.x, box.width);
  const y = box.y + across(rect.y, box.height);
  return {
    x,
    y,
    width: Math.max(0, box.x + across(rect.x + rect.w, box.width) - x),
    height: Math.max(0, box.y + across(rect.y + rect.h, box.height) - y),
  };
}

/** A rectangle in a box's pixels, in units of the box. */
export function pxToUnitRect(rect: Rect, box: Rect): UnitRect {
  const within = (value: number, extent: number) =>
    extent <= 0 ? 0 : Math.min(UNITS, Math.max(0, Math.round((value * UNITS) / extent)));
  const x = within(rect.x - box.x, box.width);
  const y = within(rect.y - box.y, box.height);
  return {
    x,
    y,
    w: Math.max(0, within(rect.x - box.x + rect.width, box.width) - x),
    h: Math.max(0, within(rect.y - box.y + rect.height, box.height) - y),
  };
}

/**
 * The ratios of a split after its divider `index` was dragged by `deltaPx` along the
 * split's direction: towards the end grows the child before the divider.
 *
 * Only the two children either side of the divider change, and together they keep the
 * share they had, so the ratios still add up to a whole. Neither is made smaller than
 * `minPx`, nor than the smallest share a child may have; where the two cannot both be
 * `minPx`, the smallest share alone is kept. `extentPx` is the split's whole extent
 * along its direction, gaps included.
 */
export function dragRatios(
  ratios: readonly number[],
  index: number,
  deltaPx: number,
  extentPx: number,
  gapPx: number,
  minPx: number,
): number[] {
  const next = [...ratios];
  const before = ratios[index];
  const after = ratios[index + 1];
  const available = extentPx - gapPx * (ratios.length - 1);
  if (before === undefined || after === undefined || !(available > 0)) return next;

  const pair = before + after;
  let least = Math.max(SHARE_MIN, Math.ceil((minPx * UNITS) / available));
  if (2 * least > pair) least = SHARE_MIN;
  // Two children that share less than two smallest shares have no room to move in.
  if (2 * least > pair) return next;

  const wanted = before + Math.round((deltaPx * UNITS) / available);
  const grown = Math.min(Math.max(wanted, least), pair - least);
  next[index] = grown;
  next[index + 1] = pair - grown;
  return next;
}

/** Ratios that give each of `count` children the same share, adding up to a whole. */
export function equalRatios(count: number): number[] {
  if (count <= 0) return [];
  const share = Math.floor(UNITS / count);
  const ratios = new Array<number>(count).fill(share);
  // What rounding lost goes to the last, as it does when a split is laid out.
  ratios[count - 1] = UNITS - share * (count - 1);
  return ratios;
}
