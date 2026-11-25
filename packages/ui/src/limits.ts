/**
 * The limits the shell puts on what a program in a terminal can make it do or hold.
 *
 * A program's output is not trusted: it can be the contents of a file someone was tricked
 * into printing. Each limit here bounds one thing output can ask for. Each value is
 * defined here and nowhere else.
 */

/**
 * The most lines a terminal keeps after they scroll off the screen, whatever the
 * configuration asks for. It is the most the configuration may ask for.
 */
export const SCROLLBACK_MAX_LINES = 100_000;

/** How many lines a terminal keeps when the configuration does not say. */
export const SCROLLBACK_DEFAULT_LINES = 10_000;

/**
 * How many of the newest lines a window keeps while older ones are fetched from the
 * session host as needed, and for how many seconds it keeps the lines it fetched once
 * it is back at the newest output, when the configuration does not say.
 */
export const WINDOW_HISTORY_DEFAULT_LINES = 1_000;
export const HISTORY_RELEASE_DEFAULT_SECONDS = 60;

/**
 * After how many seconds out of sight a pane lets go of its copy of the screen, when
 * the configuration turns that on and does not say how long.
 */
export const HIDDEN_RELEASE_DEFAULT_SECONDS = 30;

/**
 * How much memory those lines may take. Both limits apply: long or heavily styled lines
 * reach this one first. It holds the most lines there may be at a width of more than
 * two hundred columns. Every terminal of a window shares one memory that grows and is
 * never given back, so this is also what one terminal can cost the window for good.
 */
export const SCROLLBACK_MAX_BYTES = 192 * 1024 * 1024;

/**
 * The most cells of a terminal's screen. A larger one is shown as large as fits: each
 * cell costs the page about a hundred bytes where it is drawn.
 */
export const TERMINAL_CELLS_MAX = 512 * 1024;

/**
 * The most cells the terminals of servers have together in a window, and how many each
 * may have however little is left. A pane that fills a large screen with small text has
 * about a hundred thousand.
 */
export const SERVED_CELLS_MAX = 2 * 1024 * 1024;
export const SERVED_CELLS_FLOOR = 80 * 24;

/**
 * How much memory the scrollback of servers' terminals may take together in a window,
 * which part of what is left of it each may have, and what each has however little is
 * left.
 */
export const SERVED_SCROLLBACK_MAX_BYTES = 1024 * 1024 * 1024;
export const SERVED_SCROLLBACK_SHARE = 16;
export const SERVED_SCROLLBACK_FLOOR_BYTES = 2 * 1024 * 1024;

/** How many terminals of servers a window shows at once. */
export const SERVED_TERMINALS_MAX = 256;

/**
 * How much memory the engine may hold for another terminal of a server to be made in
 * it. The engine can hold twice as much: the rest is for the window's own terminals.
 */
export const SERVED_ENGINE_MAX_BYTES = 2 * 1024 * 1024 * 1024;

/**
 * The largest text a program may put on the clipboard. Programs may write the clipboard
 * and may never read it.
 */
export const OSC52_WRITE_MAX_BYTES = 1024 * 1024;

/**
 * The longest text from outside that is shown on one line, such as the title a program
 * sets, in bytes of UTF-8. The rest is cut off. The core cuts what servers say as short.
 */
export const TEXT_MAX_BYTES = 1024;

/**
 * The most status records a pane keeps of its programs. The core keeps as many; a
 * record that would be one too many pushes out the one reported longest ago.
 */
export const STATUS_RECORDS_MAX = 64;

/** The kinds of link the shell will open. Anything else in a link is only text. */
export const LINK_SCHEMES = ["http", "https", "file"] as const;

/** A kind of link the shell will open. */
export type LinkScheme = (typeof LINK_SCHEMES)[number];

/** The kinds of link that are opened only after the user has confirmed the target. */
export const LINK_SCHEMES_TO_CONFIRM: readonly LinkScheme[] = ["file"];

/**
 * The largest paste, in bytes of UTF-8. A session stops taking input it cannot hand to
 * its program once a few megabytes are waiting, so a larger paste would be cut short
 * without anyone being told.
 */
export const PASTE_MAX_BYTES = 2 * 1024 * 1024;

/** How much of a paste is shown when the user is asked to confirm it. */
export const PASTE_PREVIEW_CHARS = 2000;
