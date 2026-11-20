/**
 * What the application is, written out for a report of a fault.
 *
 * The text is made only from what `about_info` says: versions, and words for kinds of
 * things. It is given nothing else to write, so it cannot name the machine, the user,
 * a host, a folder, a workspace or a session, and it holds no credential. Whoever
 * copies it can paste it where others read it. Nothing here touches the page, so it is
 * tested with plain objects.
 */

import type { AboutInfo } from "../core/types.ts";

/** What stands for something that is not there. */
const NONE = "not installed";

/** How the session host runs, in a word or two. */
export function hostMode(host: NonNullable<AboutInfo["host"]>): string {
  return host.daemon ? "on its own" : "inside the application";
}

/** The lines of the report, each a name and what it is. */
export function debugLines(info: AboutInfo): readonly (readonly [string, string])[] {
  const host = info.host;
  return [
    ["Lume", info.appVersion],
    ["Rex server", info.rexVersion ?? NONE],
    [
      "Session host",
      host
        ? `${host.version} (${hostMode(host)}, ${host.connected ? "connected" : "not connected"})`
        : "none",
    ],
    ["Host protocol", host ? `${host.proto} (application speaks ${info.proto})` : `${info.proto}`],
    ["Webview", `${info.engine} ${info.engineVersion ?? "of an unknown version"}`],
    ["System", `${info.os} ${info.arch}`],
  ];
}

/** The report as text, one line for each thing, to be pasted into a report of a fault. */
export function debugInfo(info: AboutInfo): string {
  return debugLines(info)
    .map(([name, value]) => `${name}: ${value}`)
    .join("\n");
}
