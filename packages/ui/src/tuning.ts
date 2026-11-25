/**
 * The shell's tuning values. Each is defined here and nowhere else.
 */

/** How long a blinking cursor stays on, and then off. */
export const CURSOR_BLINK_INTERVAL_MS = 600;

/**
 * How long a terminal's size must stay other than the one its view asked for before
 * the view counts it as sized by another view, and draws it in its middle with a mark.
 * Shorter than this is a size request still on its way back.
 */
export const LETTERBOX_SETTLE_MS = 250;

/**
 * How long a view that let go of its terminal waits for the copy of the screen it asked
 * for when shown again before it draws what it has.
 */
export const RESTORE_WAIT_MS = 1000;

/**
 * How often, at most, a view tells its session about a new size. Dragging a window edge
 * changes the size on every frame; the session hears about the last one of each interval.
 */
export const REPORT_INTERVAL_MS = 16;

/**
 * How long a terminal keeps showing its last whole screen for a program that said it
 * is drawing the next one and has not said it is done (synchronized output). A program
 * that redraws says so within a frame or two; one that never does must not leave the
 * pane standing still, so after this long the screen is drawn as it is.
 */
export const SYNC_HOLD_MAX_MS = 150;

/** The longest time between two presses that still make a double or triple click. */
export const CLICK_REPEAT_INTERVAL_MS = 500;

/** How far apart two such presses may be, in cells. */
export const CLICK_REPEAT_DISTANCE_CELLS = 1;

/** How often the view scrolls by a row while a selection is dragged past its edge. */
export const AUTOSCROLL_INTERVAL_MS = 50;

/**
 * How much one step of zooming changes the font size, and how far it can go: as far as
 * the configuration's `font.size` may be set.
 */
export const FONT_ZOOM_STEP_PX = 1;
export const FONT_SIZE_MIN_PX = 1;
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

/**
 * How long a starting window waits for a terminal's first frame before it asks the GPU
 * for a device anyway: a window with no terminal on show draws none.
 */
export const GPU_ASK_LATEST_MS = 1500;

/**
 * How near an edge of a card, as a share of the card's extent, a dragged card must be
 * dropped to go beside the card on that side. Dropped nearer the middle, the two cards
 * change places.
 */
export const DROP_EDGE_FRACTION = 0.25;

/** The smallest a floating pane may be made by dragging its edges, in CSS pixels. */
export const FLOAT_MIN_PX = { width: 240, height: 120 } as const;

/**
 * How often, at most, a floating pane being moved or resized tells the core where it
 * is. The page follows the pointer on every frame.
 */
export const FLOAT_INTENT_INTERVAL_MS = 50;

/** The narrowest and the widest a dock area may be dragged, in CSS pixels. */
export const DOCK_SIZE_MIN_PX = 120;
export const DOCK_SIZE_MAX_PX = 800;

/** How often, at most, a dock area being resized tells the core its new size. */
export const DOCK_INTENT_INTERVAL_MS = 50;

/**
 * How long something that was dragged and let go keeps the place the pointer left it
 * in while the core's answer has not arrived. Past this the layout the core has is
 * shown.
 */
export const DRAG_SETTLE_TIMEOUT_MS = 1000;

/**
 * How far from its left end the top bar's contents begin on a Mac whose window draws
 * no title bar of its own: the system's three window buttons sit there.
 */
export const TRAFFIC_LIGHTS_INSET_PX = 78;

/** The most rows the command palette lists at once. */
export const PALETTE_RESULTS_MAX = 50;

/** How many of the commands last run from the palette it remembers and lists first. */
export const PALETTE_RECENTS_MAX = 8;

/** How many pictures of its panes' programs a tab shows, one over another. */
export const TAB_ICONS_MAX = 3;

/** The places a program's tile is shown. */
export type TilePlace = "tab" | "header" | "tray";

/**
 * How big a program's tile is in each place, in pixels: a little wider than tall, with
 * its drawing in the middle.
 */
export const TILE_SIZES: Readonly<
  Record<TilePlace, { readonly width: number; readonly height: number; readonly glyph: number }>
> = {
  tab: { width: 18, height: 14, glyph: 10 },
  header: { width: 20, height: 16, glyph: 11 },
  tray: { width: 16, height: 12, glyph: 9 },
};

/** How long a notice stays before it goes by itself: one that only informs. */
export const NOTICE_INFO_MS = 5000;

/** How long a warning or an error stays before it goes by itself. */
export const NOTICE_PROBLEM_MS = 12_000;

/** The most notices on show at once. An older one makes way for a newer. */
export const NOTICES_MAX = 4;

/**
 * How far from a side of the window, as a share of the window's extent that way, a
 * panel that is being dragged must be let go to be docked at that side.
 */
export const PANEL_DROP_FRACTION = 1 / 3;

/** How far one press of an arrow key moves a divider or the edge of a dock area. */
export const SEPARATOR_KEY_STEP_PX = 16;

/**
 * How long a search works at a stretch before it lets the page do something else, in
 * milliseconds. A search of a long scrollback is many such stretches.
 */
export const SEARCH_SLICE_MS = 4;

/**
 * The shortest time between two searches of what a terminal has newly printed, and how
 * long a search waits after its terminal was resized before it starts again.
 */
export const SEARCH_REFEED_INTERVAL_MS = 100;

/** How long after the last key typed into a search bar the search begins. */
export const SEARCH_QUERY_DEBOUNCE_MS = 120;

/**
 * The most matches a search counts. What a search costs to keep up to date with its
 * terminal, in time and in memory, grows with what it has found, and a single letter
 * can be found in a long scrollback millions of times.
 */
export const SEARCH_MATCHES_MAX = 10_000;

/**
 * How many rows at the top of a pane the search bar may cover. A match that is selected
 * there is scrolled to the middle of the pane, as one outside the viewport is.
 */
export const SEARCH_CLEAR_ROWS = 2;

/**
 * How long a part of the window that could not be drawn waits before something that has
 * changed has it drawn again: the first time, and the longest after it failed again and
 * again.
 */
export const PART_RETRY_MIN_MS = 1_000;
export const PART_RETRY_MAX_MS = 30_000;

/**
 * How long after the pointer pressed something among the tabs a change of them is
 * taken to be its doing, and moves. Changes made from the keyboard jump.
 */
export const TAB_POINTER_RECENT_MS = 1_000;

/** How long the mark of the tab on show keeps up with tabs the style sheet is moving. */
export const TAB_FOLLOW_LAYOUT_MS = 280;

/** How long an element that has faded out waits, hidden, for the row of tabs to change. */
export const TAB_EXIT_HELD_MS = 800;

/** How long after a new group's label has grown in its editor opens. */
export const GROUP_EDITOR_DELAY_MS = 180;

/** How long a group's name rests after a key before it is written. */
export const GROUP_NAME_WRITE_MS = 300;

/**
 * How full a terminal's scrollback must be, as a share of what it keeps while older lines
 * are fetched as needed, before scrolling to its top fetches them. The library keeps
 * whole pages, so a full scrollback can be a little short of the count.
 */
export const HISTORY_FULL_SHARE = 0.9;
