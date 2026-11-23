/**
 * The Rex servers and their sessions as the command palette finds them, and what
 * choosing one runs.
 *
 * Nothing here touches the page, so all of it is tested with plain objects.
 */

import type { RexHosts } from "../core/types.ts";
import {
  connected,
  linkOf,
  nearness,
  sessionName,
  sessionOnShow,
  standing,
  usable,
  type RexLink,
} from "../rex/model.ts";

/** A server, or one of its sessions. */
export interface RexTarget {
  readonly key: string;
  readonly kind: "host" | "session";
  readonly text: string;
  /** The quieter line: how the server stands, or which server the session is on. */
  readonly sub: string;
  readonly link: RexLink;
  /** A word at the row's end. Empty where there is none. */
  readonly meta: string;
  readonly command: string;
  readonly args: Readonly<Record<string, string>>;
}

/**
 * Each server, with its sessions after it. A server that has been shown before shows
 * the session last on show; one that answers and has none is given one. A server whose
 * address cannot be used is nothing to choose. `backend` is whose layout is on show.
 */
export function rexTargets(list: RexHosts | null, backend: string | null): RexTarget[] {
  if (!list) return [];
  return list.hosts.filter(usable).flatMap((host): RexTarget[] => {
    const link = linkOf(host);
    const shown = sessionOnShow(host, backend);
    const makes = connected(host) && host.sessions.length === 0;
    return [
      {
        key: `rex:${host.id}`,
        kind: "host",
        text: host.label,
        sub: standing(host),
        link,
        meta: nearness(host),
        command: makes ? "lume.rex.new-session" : "lume.rex.open",
        args: { host: host.id },
      },
      ...host.sessions.map((session): RexTarget => ({
        key: `rex:${host.id}:${session.id}`,
        kind: "session",
        text: sessionName(session),
        sub: host.label,
        link,
        meta: session.id === shown ? "attached" : "session",
        command: "lume.rex.open",
        args: { host: host.id, session: session.id },
      })),
    ];
  });
}
