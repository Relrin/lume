/**
 * The shapes of the cursor's effects at a moment, worked out on the CPU.
 *
 * A trail is the outline round where the cursor was and where it is going, each of
 * which moves from the old place to the new along its own curve. The outline is the
 * convex hull of the two, so it is one shape however the cursor jumps, and the shader
 * only has to say how far each pixel is from it. Everything here is in CSS pixels with
 * the origin at the top left, and nothing touches the page, so all of it is tested with
 * plain numbers.
 */

import { easeOutCirc, easeOutCubic, fadeOutQuad, smoothstep } from "./easing.ts";
import {
  COMET_SHORT_SHARE,
  COMET_TAIL_DELAY,
  COMET_TAIL_SCALE,
  FX_POLYGON_MAX,
  PULSE_REACH,
  PULSE_RING,
  SWEEP_START,
  TRAIL_MIN_DISTANCE,
  WARP_TRAIL_SIZE,
} from "./tuning.ts";

/** A rectangle in CSS pixels: a cursor's cell, or what of it the cursor covers. */
export interface FxRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface Point {
  x: number;
  y: number;
}

/** The four corners of a shape: top left, top right, bottom right, bottom left. */
export type Corners = [Point, Point, Point, Point];

/** What the cursor leaves behind when it jumps. */
export type TrailKind = "none" | "warp" | "sweep" | "comet";

/** What the cursor gives off when it changes shape or lands in another pane. */
export type PulseKind = "none" | "burst" | "ripple" | "box-burst" | "box-ripple";

export function cornersOf(rect: FxRect): Corners {
  const right = rect.x + rect.width;
  const bottom = rect.y + rect.height;
  return [
    { x: rect.x, y: rect.y },
    { x: right, y: rect.y },
    { x: right, y: bottom },
    { x: rect.x, y: bottom },
  ];
}

export function centreOf(corners: Corners): Point {
  let x = 0;
  let y = 0;
  for (const corner of corners) {
    x += corner.x;
    y += corner.y;
  }
  return { x: x / 4, y: y / 4 };
}

/** Each corner `amounts[i]` of the way from `from` to `to`. */
export function lerpCorners(
  from: Corners,
  to: Corners,
  amounts: readonly [number, number, number, number],
): Corners {
  const at = (index: 0 | 1 | 2 | 3): Point => {
    const a = from[index];
    const b = to[index];
    const t = amounts[index];
    return { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
  };
  return [at(0), at(1), at(2), at(3)];
}

/** The corners drawn nearer their centre: `scale` 1 leaves them where they are. */
export function scaleCorners(corners: Corners, scale: number): Corners {
  const centre = centreOf(corners);
  const toward = (point: Point): Point => ({
    x: centre.x + (point.x - centre.x) * scale,
    y: centre.y + (point.y - centre.y) * scale,
  });
  return [toward(corners[0]), toward(corners[1]), toward(corners[2]), toward(corners[3])];
}

/**
 * Writes the convex hull of `points` into `out` as x, y pairs going round one way, and
 * says how many corners it has. Points that repeat or lie on an edge are left out, so
 * no edge has no length.
 */
export function convexHull(points: readonly Point[], out: Float32Array): number {
  const sorted = [...points].sort((a, b) => a.x - b.x || a.y - b.y);
  const cross = (o: Point, a: Point, b: Point) =>
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
  const hull: Point[] = [];
  // Andrew's monotone chain: the lower half, then the upper.
  for (const pass of [sorted, [...sorted].reverse()]) {
    const start = hull.length;
    for (const point of pass) {
      while (
        hull.length >= start + 2 &&
        cross(hull[hull.length - 2] as Point, hull[hull.length - 1] as Point, point) <= 1e-6
      ) {
        hull.pop();
      }
      hull.push(point);
    }
    // The last point of each half is the first of the other.
    hull.pop();
  }
  const count = Math.min(hull.length, FX_POLYGON_MAX, out.length >> 1);
  for (let index = 0; index < count; index++) {
    const point = hull[index] as Point;
    out[index * 2] = point.x;
    out[index * 2 + 1] = point.y;
  }
  return count;
}

/** Whether a jump from `from` to `to` is far enough to leave a trail. */
export function farEnough(from: Corners, to: FxRect): boolean {
  const a = centreOf(from);
  const b = centreOf(cornersOf(to));
  return Math.hypot(b.x - a.x, b.y - a.y) > TRAIL_MIN_DISTANCE * to.height;
}

/**
 * How long each corner of a warp takes, as shares of the whole: the corners that lead
 * the jump arrive first, those that trail it last, and those at the sides in between.
 */
export function warpShares(from: Corners, to: FxRect): [number, number, number, number] {
  const a = centreOf(from);
  const b = centreOf(cornersOf(to));
  const sign = (value: number) => (Math.abs(value) < 0.5 ? 0 : Math.sign(value));
  const moveX = sign(b.x - a.x);
  const moveY = sign(b.y - a.y);
  const lead = 1 - WARP_TRAIL_SIZE;
  const side = (lead + 1) / 2;
  // The direction of each corner from the centre: top left, top right, bottom right,
  // bottom left.
  const diagonals = [
    [-1, -1],
    [1, -1],
    [1, 1],
    [-1, 1],
  ] as const;
  const share = (index: 0 | 1 | 2 | 3): number => {
    const [dx, dy] = diagonals[index];
    const along = dx * moveX + dy * moveY;
    return along > 0.5 ? lead : along >= -0.5 ? side : 1;
  };
  return [share(0), share(1), share(2), share(3)];
}

/** A trail under way. */
export interface Trail {
  readonly kind: Exclude<TrailKind, "none">;
  /** When it started, and how long it lasts, in milliseconds. */
  readonly start: number;
  readonly duration: number;
  /** Where the cursor was: the corners of the shape it leaves from. */
  readonly from: Corners;
  readonly to: FxRect;
  readonly toCorners: Corners;
  /** For a warp, how long each corner takes, as a share of the whole. */
  readonly shares: readonly [number, number, number, number];
  /** For a comet, whether the jump is short enough for the head to fly too. */
  readonly short: boolean;
}

/** Starts a trail from `from` to `to`. `height` is the height of the window. */
export function startTrail(
  kind: Exclude<TrailKind, "none">,
  from: Corners,
  to: FxRect,
  start: number,
  duration: number,
  height: number,
): Trail {
  const toCorners = cornersOf(to);
  const a = centreOf(from);
  const b = centreOf(toCorners);
  const length = Math.hypot(b.x - a.x, b.y - a.y);
  const longest = COMET_SHORT_SHARE * height;
  return {
    kind,
    start,
    duration,
    from,
    to,
    toCorners,
    shares: warpShares(from, to),
    short: length < longest,
  };
}

/** What a trail looks like at a moment. */
export interface TrailShape {
  /** How many corners of the outline were written. */
  readonly count: number;
  /** The middles of the tail and of the head, which a fading trail fades between. */
  readonly tail: Point;
  readonly head: Point;
  /** Where the trail's tail is: where a trail that starts now leaves from. */
  readonly trailing: Corners;
  /** Whether the trail has arrived and has nothing more to show. */
  readonly done: boolean;
}

/** The outline of `trail` at `now`, written into `out`. */
export function trailShape(trail: Trail, now: number, out: Float32Array): TrailShape {
  const t = trail.duration > 0 ? (now - trail.start) / trail.duration : 1;
  const done = t >= 1;
  let head: Corners;
  let tail: Corners;
  switch (trail.kind) {
    case "warp": {
      const [a, b, c, d] = trail.shares;
      const corners = lerpCorners(trail.from, trail.toCorners, [
        easeOutCirc(t / a),
        easeOutCirc(t / b),
        easeOutCirc(t / c),
        easeOutCirc(t / d),
      ]);
      head = corners;
      tail = corners;
      break;
    }
    case "sweep": {
      const along = SWEEP_START + (1 - SWEEP_START) * easeOutCubic(t);
      head = trail.toCorners;
      tail = lerpCorners(trail.from, trail.toCorners, [along, along, along, along]);
      break;
    }
    case "comet": {
      const headAt = trail.short ? easeOutCirc(t) : 1;
      const tailAt = trail.short ? easeOutCirc(smoothstep(COMET_TAIL_DELAY, 1, t)) : easeOutCirc(t);
      head = lerpCorners(trail.from, trail.toCorners, [headAt, headAt, headAt, headAt]);
      tail = lerpCorners(trail.from, trail.toCorners, [tailAt, tailAt, tailAt, tailAt]);
      break;
    }
  }
  const drawnTail = trail.kind === "comet" ? scaleCorners(tail, COMET_TAIL_SCALE) : tail;
  const count = convexHull(trail.kind === "warp" ? head : [...head, ...drawnTail], out);
  return {
    count,
    tail: centreOf(tail),
    head: centreOf(head),
    trailing: tail,
    done,
  };
}

/** A pulse under way. */
export interface Pulse {
  readonly kind: Exclude<PulseKind, "none">;
  readonly start: number;
  readonly duration: number;
  readonly at: FxRect;
}

/** What a pulse looks like at a moment. */
export interface PulseShape {
  /** The middle of the cursor it grows from. */
  readonly centre: Point;
  /** For a disc or a ring, its radius; for a box, half its width and height. */
  readonly radius: number;
  readonly halfWidth: number;
  readonly halfHeight: number;
  /** How thick a ring or an outline is; 0 for a filled shape. */
  readonly ring: number;
  /** How much of it is left, from 1 at the start to 0. */
  readonly fade: number;
  readonly done: boolean;
}

export function pulseShape(pulse: Pulse, now: number): PulseShape {
  const t = pulse.duration > 0 ? (now - pulse.start) / pulse.duration : 1;
  const grown = easeOutCirc(t) * PULSE_REACH * pulse.at.height;
  const ringed = pulse.kind === "ripple" || pulse.kind === "box-ripple";
  return {
    centre: { x: pulse.at.x + pulse.at.width / 2, y: pulse.at.y + pulse.at.height / 2 },
    radius: grown,
    halfWidth: pulse.at.width / 2 + grown,
    halfHeight: pulse.at.height / 2 + grown,
    ring: ringed ? PULSE_RING * pulse.at.height : 0,
    fade: fadeOutQuad(t),
    done: t >= 1,
  };
}
