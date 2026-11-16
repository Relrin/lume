/**
 * Arithmetic on colours packed as a frame packs them: R, G, B, A in memory order.
 */

/**
 * The colour of `top` laid over `bottom`. Worked out once here, not left to each tier's
 * own blending, so that a translucent colour over a cell looks the same on all of them.
 */
export function blendOver(top: number, bottom: number): number {
  const topAlpha = (top >>> 24) / 255;
  if (topAlpha === 1) return top;
  if (topAlpha === 0) return bottom;
  const bottomAlpha = ((bottom >>> 24) / 255) * (1 - topAlpha);
  const alpha = topAlpha + bottomAlpha;
  const channel = (shift: number) =>
    Math.round(
      (((top >>> shift) & 0xff) * topAlpha + ((bottom >>> shift) & 0xff) * bottomAlpha) / alpha,
    );
  return (
    (channel(0) | (channel(8) << 8) | (channel(16) << 16) | (Math.round(alpha * 255) << 24)) >>> 0
  );
}
