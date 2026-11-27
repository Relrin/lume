/**
 * How the cursor's effects look and move. Each value is defined here and nowhere else.
 *
 * Lengths are in CSS pixels or in cursor heights, times in milliseconds.
 */

/** How long each trail lasts unless the settings say otherwise. */
export const WARP_MS = 200;
export const SWEEP_MS = 200;
export const COMET_MS = 160;

/**
 * How long each motion of the cursor takes unless the settings say otherwise: short,
 * so the cursor is in its place before the next key.
 */
export const GLIDE_MS = 120;
export const GLIDE_SLOW_MS = 220;

/** How long each pulse lasts unless the settings say otherwise. */
export const PULSE_MS = 150;

/**
 * How far the cursor must jump for a trail, in cursor heights between the old and the
 * new centre. Typing moves it one cell at a time, which is never far enough.
 */
export const TRAIL_MIN_DISTANCE = 1.5;

/**
 * How much sooner a warp's leading corners arrive than its trailing ones, from 0 (all
 * together: the cursor slides) to 1 (the leading corners jump at once). The corners
 * at the sides arrive halfway between.
 */
export const WARP_TRAIL_SIZE = 0.8;

/** Where a sweep's band starts: this share of the way from the old place to the new. */
export const SWEEP_START = 0.5;

/**
 * A comet whose jump is shorter than this share of the window's height flies its head
 * across too; a longer one puts its head in the new place at once and lets the tail
 * follow, so a jump across the window is never slow.
 */
export const COMET_SHORT_SHARE = 0.1;

/**
 * How long a short comet's tail waits before it follows the head, as a share of the
 * comet's time: the comet stretches out, then draws in behind the head.
 */
export const COMET_TAIL_DELAY = 0.3;

/** How wide a comet's tail is beside its head. */
export const COMET_TAIL_SCALE = 0.35;

/** How sharply a comet fades towards its tail: the fade along it is raised to this. */
export const COMET_FADE_POWER = 1.5;

/** The glow round a comet: how far it reaches, in CSS pixels, and how bright it starts. */
export const COMET_HALO_RADIUS = 6;
export const COMET_HALO_STRENGTH = 0.45;

/** How far a pulse grows, in cursor heights past the cursor's own size. */
export const PULSE_REACH = 1.6;

/** How thick a ripple's ring is, in cursor heights. */
export const PULSE_RING = 0.18;

/** How opaque a pulse starts. It is quieter than a trail, which marks where to look. */
export const PULSE_ALPHA = 0.55;

/** How wide the edge of a shape is smoothed, in device pixels. */
export const FX_EDGE = 1;

/** How many corners a trail's outline may have: the hull of two four-cornered shapes. */
export const FX_POLYGON_MAX = 8;

/**
 * How much of the first frame of an effect has already passed when it starts. The
 * frame that finds the jump is drawn a moment later, so the effect starts moving in
 * that very frame instead of the next.
 */
export const FX_FIRST_FRAME_MS = 1000 / 120;
