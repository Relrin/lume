/**
 * The limits the shell puts on what a program in a terminal can make it do or hold.
 *
 * A program's output is not trusted: it can be the contents of a file someone was tricked
 * into printing. Each limit here bounds one thing output can ask for. Each value is
 * defined here and nowhere else.
 */

/** How many lines a terminal keeps after they scroll off the screen. */
export const SCROLLBACK_MAX_LINES = 10_000;

/**
 * How much memory those lines may take. Both limits apply: long or heavily styled lines
 * reach this one first.
 */
export const SCROLLBACK_MAX_BYTES = 64 * 1024 * 1024;

/**
 * The largest text a program may put on the clipboard. Programs may write the clipboard
 * and may never read it.
 */
export const OSC52_WRITE_MAX_BYTES = 1024 * 1024;

/** The longest window title a program may set, in bytes of UTF-8. The rest is cut off. */
export const TITLE_MAX_BYTES = 1024;

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
