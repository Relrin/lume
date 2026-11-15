/**
 * Box-drawing characters and block elements as shapes.
 *
 * A font's own glyphs for these rarely fill the cell exactly, which leaves hairline gaps
 * where lines should join, more so once the line height is not the font's own. Drawing
 * them from the cell size makes every line reach the cell edge and every block fill its
 * share of the cell.
 *
 * Covers U+2500 to U+259F: lines and corners in light, heavy and double weights, dashed
 * lines, rounded corners, diagonals, and the block and shade elements.
 */

import { BOX_HEAVY_FACTOR, SHADE_ALPHAS } from "../constants.ts";
import type { Shape } from "./coverage.ts";

const FIRST = 0x2500;
const LAST = 0x259f;

/** Whether a codepoint is drawn from shapes instead of from the font. */
export function isProcedural(codepoint: number): boolean {
  return codepoint >= FIRST && codepoint <= LAST;
}

// The weight of a line: none, light, heavy or double.
const NONE = 0;
const HEAVY = 2;
const DOUBLE = 3;

/**
 * The arms of U+2500 to U+257F that are made only of straight lines to the cell edges,
 * as four digits: the weight of the arm going up, right, down and left. Characters that
 * are not such a junction (dashes, arcs, diagonals) are left out and handled apart.
 */
const ARMS: Readonly<Record<number, string>> = {
  0x2500: "0101",
  0x2501: "0202",
  0x2502: "1010",
  0x2503: "2020",
  0x250c: "0110",
  0x250d: "0210",
  0x250e: "0120",
  0x250f: "0220",
  0x2510: "0011",
  0x2511: "0012",
  0x2512: "0021",
  0x2513: "0022",
  0x2514: "1100",
  0x2515: "1200",
  0x2516: "2100",
  0x2517: "2200",
  0x2518: "1001",
  0x2519: "1002",
  0x251a: "2001",
  0x251b: "2002",
  0x251c: "1110",
  0x251d: "1210",
  0x251e: "2110",
  0x251f: "1120",
  0x2520: "2120",
  0x2521: "2210",
  0x2522: "1220",
  0x2523: "2220",
  0x2524: "1011",
  0x2525: "1012",
  0x2526: "2011",
  0x2527: "1021",
  0x2528: "2021",
  0x2529: "2012",
  0x252a: "1022",
  0x252b: "2022",
  0x252c: "0111",
  0x252d: "0112",
  0x252e: "0211",
  0x252f: "0212",
  0x2530: "0121",
  0x2531: "0122",
  0x2532: "0221",
  0x2533: "0222",
  0x2534: "1101",
  0x2535: "1102",
  0x2536: "1201",
  0x2537: "1202",
  0x2538: "2101",
  0x2539: "2102",
  0x253a: "2201",
  0x253b: "2202",
  0x253c: "1111",
  0x253d: "1112",
  0x253e: "1211",
  0x253f: "1212",
  0x2540: "2111",
  0x2541: "1121",
  0x2542: "2121",
  0x2543: "2112",
  0x2544: "2211",
  0x2545: "1122",
  0x2546: "1221",
  0x2547: "2212",
  0x2548: "1222",
  0x2549: "2122",
  0x254a: "2221",
  0x254b: "2222",
  0x2550: "0303",
  0x2551: "3030",
  0x2552: "0310",
  0x2553: "0130",
  0x2554: "0330",
  0x2555: "0013",
  0x2556: "0031",
  0x2557: "0033",
  0x2558: "1300",
  0x2559: "3100",
  0x255a: "3300",
  0x255b: "1003",
  0x255c: "3001",
  0x255d: "3003",
  0x255e: "1310",
  0x255f: "3130",
  0x2560: "3330",
  0x2561: "1013",
  0x2562: "3031",
  0x2563: "3033",
  0x2564: "0313",
  0x2565: "0131",
  0x2566: "0333",
  0x2567: "1303",
  0x2568: "3101",
  0x2569: "3303",
  0x256a: "1313",
  0x256b: "3131",
  0x256c: "3333",
  0x2574: "0001",
  0x2575: "1000",
  0x2576: "0100",
  0x2577: "0010",
  0x2578: "0002",
  0x2579: "2000",
  0x257a: "0200",
  0x257b: "0020",
  0x257c: "0201",
  0x257d: "1020",
  0x257e: "0102",
  0x257f: "2010",
};

/** Dashed lines: whether horizontal, how many dashes, and whether heavy. */
const DASHES: Readonly<
  Record<number, readonly [horizontal: boolean, count: number, heavy: boolean]>
> = {
  0x2504: [true, 3, false],
  0x2505: [true, 3, true],
  0x2506: [false, 3, false],
  0x2507: [false, 3, true],
  0x2508: [true, 4, false],
  0x2509: [true, 4, true],
  0x250a: [false, 4, false],
  0x250b: [false, 4, true],
  0x254c: [true, 2, false],
  0x254d: [true, 2, true],
  0x254e: [false, 2, false],
  0x254f: [false, 2, true],
};

/**
 * The shapes of a box-drawing or block character for a cell of `width` by `height`
 * pixels, where `thickness` is the width of a light line. `undefined` for a codepoint
 * outside the range.
 */
export function boxShapes(
  codepoint: number,
  width: number,
  height: number,
  thickness: number,
): Shape[] | undefined {
  if (!isProcedural(codepoint)) return undefined;
  const light = Math.max(1, Math.min(thickness, width, height));
  if (codepoint >= 0x2580) return blockShapes(codepoint, width, height);

  const arms = ARMS[codepoint];
  if (arms) return junction(arms, width, height, light);
  const dash = DASHES[codepoint];
  if (dash) return dashes(dash, width, height, light);
  if (codepoint >= 0x256d && codepoint <= 0x2570) return corner(codepoint, width, height, light);
  return diagonals(codepoint, width, height, light);
}

/** How far towards and past the middle of the cell a stroke of an arm runs. */
type Reach =
  /** To the middle, where the opposite arm meets it. */
  | "middle"
  /** Up to the nearer of two crossing double lines. */
  | "near"
  /** Past the middle to the outside of a double line that turns the corner. */
  | "far"
  /** To the far side of a crossing single line, so the corner is square. */
  | { across: number };

/**
 * A junction of straight arms.
 *
 * Each arm is one stroke, or two for a double line. What needs care is where a stroke
 * ends in the middle of the cell, so that corners are square and the two lines of a
 * double line turn a corner one inside the other instead of crossing.
 */
function junction(arms: string, width: number, height: number, light: number): Shape[] {
  const [up, right, down, left] = [0, 1, 2, 3].map((i) => Number(arms.charAt(i))) as [
    number,
    number,
    number,
    number,
  ];
  const heavy = Math.min(light * BOX_HEAVY_FACTOR, width, height);
  const thick = (weight: number) => (weight === HEAVY ? heavy : light);
  const shapes: Shape[] = [];

  /**
   * Add the strokes of one arm. `extent` is the cell size along the arm and `cross` the
   * size across it; `forward` says whether the arm runs towards the far edge. `before`
   * and `after` are the perpendicular arms on the low and high side across the arm.
   */
  const arm = (
    weight: number,
    opposite: number,
    before: number,
    after: number,
    extent: number,
    cross: number,
    forward: boolean,
    add: (from: number, to: number, crossFrom: number, crossTo: number) => void,
  ) => {
    if (weight === NONE) return;
    // Where a light line through the middle of the cell starts, along the arm.
    const centre = Math.floor((extent - light) / 2);
    const end = (reach: Reach): number => {
      if (reach === "middle") return forward ? centre : centre + light;
      if (reach === "near") return forward ? centre + light : centre;
      if (reach === "far") return forward ? centre - light : centre + 2 * light;
      const start = Math.floor((extent - reach.across) / 2);
      return forward ? start : start + reach.across;
    };
    const stroke = (reach: Reach, crossFrom: number, size: number) => {
      const at = end(reach);
      if (forward) add(at, extent, crossFrom, crossFrom + size);
      else add(0, at, crossFrom, crossFrom + size);
    };
    const widest = Math.max(before ? thick(before) : 0, after ? thick(after) : 0);

    if (weight !== DOUBLE) {
      const size = thick(weight);
      let reach: Reach = "middle";
      if (opposite === NONE) {
        if (before === DOUBLE && after === DOUBLE) reach = "near";
        else if (before === DOUBLE || after === DOUBLE) reach = "far";
        else if (widest > 0) reach = { across: widest };
      }
      stroke(reach, Math.floor((cross - size) / 2), size);
      return;
    }

    // A double line: two light strokes one thickness apart.
    const middle = Math.floor((cross - light) / 2);
    const side = (near: number, other: number): Reach => {
      if (near === DOUBLE) return "near";
      if (other === DOUBLE) return "far";
      if (widest > 0) return { across: widest };
      return "middle";
    };
    stroke(side(before, after), middle - light, light);
    stroke(side(after, before), middle + light, light);
  };

  const horizontal = (x0: number, x1: number, y0: number, y1: number) =>
    shapes.push({ kind: "rect", x0, y0, x1, y1 });
  const vertical = (y0: number, y1: number, x0: number, x1: number) =>
    shapes.push({ kind: "rect", x0, y0, x1, y1 });

  arm(right, left, up, down, width, height, true, horizontal);
  arm(left, right, up, down, width, height, false, horizontal);
  arm(down, up, left, right, height, width, true, vertical);
  arm(up, down, left, right, height, width, false, vertical);
  return shapes;
}

function dashes(
  [horizontal, count, heavy]: readonly [boolean, number, boolean],
  width: number,
  height: number,
  light: number,
): Shape[] {
  const size = heavy ? Math.min(light * BOX_HEAVY_FACTOR, width, height) : light;
  const extent = horizontal ? width : height;
  const cross = Math.floor(((horizontal ? height : width) - size) / 2);
  const period = extent / count;
  // The gap is split between the two ends of a dash, so dashes in neighbouring cells
  // are spaced like dashes within one.
  const gap = Math.max(1, Math.round(period / 4));
  const shapes: Shape[] = [];
  for (let i = 0; i < count; i++) {
    const from = Math.round(i * period + gap / 2);
    const to = Math.max(from + 1, Math.round((i + 1) * period - gap / 2));
    shapes.push(
      horizontal
        ? { kind: "rect", x0: from, y0: cross, x1: to, y1: cross + size }
        : { kind: "rect", x0: cross, y0: from, x1: cross + size, y1: to },
    );
  }
  return shapes;
}

/** A rounded corner: a quarter circle and the straight runs from it to the cell edges. */
function corner(codepoint: number, width: number, height: number, light: number): Shape[] {
  const left = Math.floor((width - light) / 2);
  const top = Math.floor((height - light) / 2);
  // The centre lines of the strokes, so the arc meets straight lines in the next cells.
  const cx = left + light / 2;
  const cy = top + light / 2;
  const r = Math.min(width, height) / 2;
  // Which way the corner opens: down or up, right or left.
  const down = codepoint === 0x256d || codepoint === 0x256e;
  const right = codepoint === 0x256d || codepoint === 0x2570;
  const sx = right ? 1 : -1;
  const sy = down ? 1 : -1;
  // The arc is the quarter of the circle nearest the middle of the cell.
  const quadrant = (right ? (down ? 2 : 1) : down ? 3 : 0) as 0 | 1 | 2 | 3;

  const shapes: Shape[] = [
    { kind: "arc", cx: cx + sx * r, cy: cy + sy * r, r, quadrant, width: light },
  ];
  const fromArcX = Math.round(cx + sx * r);
  if (right ? fromArcX < width : fromArcX > 0) {
    shapes.push({
      kind: "rect",
      x0: right ? fromArcX : 0,
      y0: top,
      x1: right ? width : fromArcX,
      y1: top + light,
    });
  }
  const fromArcY = Math.round(cy + sy * r);
  if (down ? fromArcY < height : fromArcY > 0) {
    shapes.push({
      kind: "rect",
      x0: left,
      y0: down ? fromArcY : 0,
      x1: left + light,
      y1: down ? height : fromArcY,
    });
  }
  return shapes;
}

function diagonals(codepoint: number, width: number, height: number, light: number): Shape[] {
  const shapes: Shape[] = [];
  // The strokes run a little past the corners so that their square ends fill them.
  const dx = (light * width) / Math.hypot(width, height);
  const dy = (light * height) / Math.hypot(width, height);
  if (codepoint === 0x2571 || codepoint === 0x2573) {
    shapes.push({
      kind: "line",
      x0: width + dx,
      y0: -dy,
      x1: -dx,
      y1: height + dy,
      width: light,
    });
  }
  if (codepoint === 0x2572 || codepoint === 0x2573) {
    shapes.push({
      kind: "line",
      x0: -dx,
      y0: -dy,
      x1: width + dx,
      y1: height + dy,
      width: light,
    });
  }
  return shapes;
}

function blockShapes(codepoint: number, width: number, height: number): Shape[] {
  const rect = (x0: number, y0: number, x1: number, y1: number, alpha?: number): Shape => ({
    kind: "rect",
    x0: Math.round(x0 * width),
    y0: Math.round(y0 * height),
    x1: Math.round(x1 * width),
    y1: Math.round(y1 * height),
    ...(alpha === undefined ? {} : { alpha }),
  });
  const quadrants = (
    upperLeft: boolean,
    upperRight: boolean,
    lowerLeft: boolean,
    lowerRight: boolean,
  ) => {
    const shapes: Shape[] = [];
    if (upperLeft) shapes.push(rect(0, 0, 0.5, 0.5));
    if (upperRight) shapes.push(rect(0.5, 0, 1, 0.5));
    if (lowerLeft) shapes.push(rect(0, 0.5, 0.5, 1));
    if (lowerRight) shapes.push(rect(0.5, 0.5, 1, 1));
    return shapes;
  };

  // U+2581 to U+2588: the lower one to eight eighths.
  if (codepoint >= 0x2581 && codepoint <= 0x2588) {
    return [rect(0, 1 - (codepoint - 0x2580) / 8, 1, 1)];
  }
  // U+2589 to U+258F: the left seven to one eighths.
  if (codepoint >= 0x2589 && codepoint <= 0x258f) {
    return [rect(0, 0, (0x2590 - codepoint) / 8, 1)];
  }
  switch (codepoint) {
    case 0x2580:
      return [rect(0, 0, 1, 0.5)];
    case 0x2590:
      return [rect(0.5, 0, 1, 1)];
    case 0x2591:
      return [rect(0, 0, 1, 1, SHADE_ALPHAS[0])];
    case 0x2592:
      return [rect(0, 0, 1, 1, SHADE_ALPHAS[1])];
    case 0x2593:
      return [rect(0, 0, 1, 1, SHADE_ALPHAS[2])];
    case 0x2594:
      return [rect(0, 0, 1, 1 / 8)];
    case 0x2595:
      return [rect(7 / 8, 0, 1, 1)];
    case 0x2596:
      return quadrants(false, false, true, false);
    case 0x2597:
      return quadrants(false, false, false, true);
    case 0x2598:
      return quadrants(true, false, false, false);
    case 0x2599:
      return quadrants(true, false, true, true);
    case 0x259a:
      return quadrants(true, false, false, true);
    case 0x259b:
      return quadrants(true, true, true, false);
    case 0x259c:
      return quadrants(true, true, false, true);
    case 0x259d:
      return quadrants(false, true, false, false);
    case 0x259e:
      return quadrants(false, true, true, false);
    default:
      return quadrants(false, true, true, true);
  }
}
