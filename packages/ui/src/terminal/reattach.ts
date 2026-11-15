/**
 * Which of the running sessions a reloaded page shows.
 *
 * A reload replaces the page and leaves its sessions running. The new page shows the
 * session the old one showed, if it is still there, and otherwise whichever it can attach
 * to. The others stay in the background.
 */

import type { SessionInfo } from "../transport/types.ts";

/**
 * The sessions in the order a reloaded page should try to attach to them: the one it
 * showed before first, then the rest as listed. Sessions whose program has ended are
 * left out: there is nothing in them to come back to.
 */
export function reattachOrder(
  sessions: readonly SessionInfo[],
  shown: string | null,
): SessionInfo[] {
  const live = sessions.filter((session) => session.exit === null);
  return [
    ...live.filter((session) => session.id === shown),
    ...live.filter((session) => session.id !== shown),
  ];
}
