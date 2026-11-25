/** The values listing Rex servers is tuned by. Each is defined here and nowhere else. */

/** How many of a server's sessions are listed at most. */
export const SESSIONS_LISTED_MAX = 200;

/**
 * How often something that lists servers and is on show asks about them again. The core
 * takes the server of this machine as looked at for longer than this.
 */
export const REX_LOOK_EVERY_MS = 20_000;
