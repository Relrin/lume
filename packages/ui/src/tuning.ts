/**
 * The shell's tuning values. Each is defined here and nowhere else.
 */

/** How long a blinking cursor stays on, and then off. */
export const CURSOR_BLINK_INTERVAL_MS = 600;

/**
 * How often, at most, a view tells its session about a new size. Dragging a window edge
 * changes the size on every frame; the session hears about the last one of each interval.
 */
export const REPORT_INTERVAL_MS = 16;

/** The longest time between two presses that still make a double or triple click. */
export const CLICK_REPEAT_INTERVAL_MS = 500;

/** How far apart two such presses may be, in cells. */
export const CLICK_REPEAT_DISTANCE_CELLS = 1;

/** How often the view scrolls by a row while a selection is dragged past its edge. */
export const AUTOSCROLL_INTERVAL_MS = 50;

/** The font size a terminal starts with, in CSS pixels. */
export const FONT_SIZE_DEFAULT_PX = 14;

/** How much one step of zooming changes the font size, and how far it can go. */
export const FONT_ZOOM_STEP_PX = 1;
export const FONT_SIZE_MIN_PX = 6;
export const FONT_SIZE_MAX_PX = 72;

/**
 * How many pixels of wheel movement scroll one line. A mouse wheel reports a hundred
 * pixels for each notch, which this turns into three lines; a touchpad reports small
 * amounts that add up.
 */
export const WHEEL_PIXELS_PER_LINE = 100 / 3;

/** How many rows either side of the pointer are joined when looking for a wrapped link. */
export const LINK_SCAN_MAX_ROWS = 4;

/**
 * Whether Option on a Mac acts as Alt. When it does not, Option types the characters the
 * keyboard layout gives it, which is what a Mac does everywhere else.
 */
export const MAC_OPTION_AS_ALT = false;
