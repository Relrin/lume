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

/**
 * How long a terminal may take to have a size it was told to take from outside the
 * window. The session answers a size request at the point of its output where the
 * program saw the size, which is at once unless the program has stopped reading. This is
 * shorter than the core waits for the page, so the page's own answer arrives first.
 */
export const PINNED_SIZE_TIMEOUT_MS = 4000;

/** How many rows either side of the pointer are joined when looking for a wrapped link. */
export const LINK_SCAN_MAX_ROWS = 4;

/**
 * Whether Option on a Mac acts as Alt. When it does not, Option types the characters the
 * keyboard layout gives it, which is what a Mac does everywhere else.
 */
export const MAC_OPTION_AS_ALT = false;

/**
 * The smallest a tiled pane may be made by dragging a divider, in CSS pixels. Below
 * this a terminal has too few cells to be of use.
 */
export const PANE_MIN_PX = { width: 120, height: 60 } as const;

/**
 * The narrowest a divider between two panes is for the pointer, in CSS pixels. A gap
 * narrower than this is widened evenly on both sides, so a thin gap can still be taken
 * hold of.
 */
export const DIVIDER_HIT_MIN_PX = 6;

/**
 * How often, at most, a divider being dragged tells the core its split's new ratios.
 * The page follows the pointer on every frame; the core hears of it this often and
 * once more when the pointer lets go.
 */
export const DIVIDER_INTENT_INTERVAL_MS = 50;

/**
 * How long the first keys of a key sequence wait for the next one. After that the
 * sequence is forgotten and the next key begins afresh.
 */
export const KEY_SEQUENCE_TIMEOUT_MS = 1000;

/**
 * How long a divider that was let go keeps its own ratios while the core's have not
 * arrived yet. The core answers in a few milliseconds; past this something went wrong,
 * and the layout the core has is shown.
 */
export const DIVIDER_SETTLE_TIMEOUT_MS = 1000;

/**
 * How far the pointer moves with a button held before that is a drag and not a click,
 * in CSS pixels.
 */
export const DRAG_START_PX = 4;

/**
 * How long the page waits for a terminal's font before it draws with the next font in
 * the list. A font the page ships arrives in milliseconds; this only keeps a start from
 * hanging on one that never does.
 */
export const FONT_LOAD_TIMEOUT_MS = 3000;
