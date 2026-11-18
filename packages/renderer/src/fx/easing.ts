/**
 * Curves an effect moves along: each takes how far through it is, from 0 to 1, and
 * gives how far along its way it is, 0 at the start and 1 at the end.
 */

export type Easing = (t: number) => number;

const clamp01 = (t: number): number => Math.min(1, Math.max(0, t));

/** Fast at first, then settling: a quarter circle. */
export const easeOutCirc: Easing = (t) => {
  const x = clamp01(t);
  return Math.sqrt(1 - (x - 1) * (x - 1));
};

/** Fast at first, then settling. */
export const easeOutCubic: Easing = (t) => {
  const x = 1 - clamp01(t);
  return 1 - x * x * x;
};

/** How a pulse fades: fully there at the start, gone at the end, quickest at first. */
export const fadeOutQuad: Easing = (t) => {
  const x = 1 - clamp01(t);
  return x * x;
};

/** A curve that runs from 0 to 1 between `from` and `to`, smoothly at both ends. */
export function smoothstep(from: number, to: number, t: number): number {
  if (to <= from) return t < from ? 0 : 1;
  const x = clamp01((t - from) / (to - from));
  return x * x * (3 - 2 * x);
}
