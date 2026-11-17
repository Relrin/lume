/**
 * Where a shell says it is, as a pane's header shows it.
 *
 * A shell reports its directory as a URL, `file://<machine>/<path>`. The page only shows
 * it: the URL is read the way the core reads it, so the two agree on what a report
 * says, and a report the core would not take is not shown either.
 */

import type { Machine } from "../core/types.ts";

/** The longest machine name the older Windows naming keeps; a longer one is cut to it. */
const NETBIOS_NAME_MAX_CHARS = 15;

/** A control character: no directory has one in its name. */
const CONTROL = /\p{Cc}/u;

/** The machine and path a `file:` URL names, the path with its escapes undone. */
export function parseFileUrl(url: string): { host: string; path: string } | undefined {
  if (url.slice(0, 7).toLowerCase() !== "file://") return undefined;
  const rest = url.slice(7);
  const slash = rest.indexOf("/");
  if (slash < 0) return undefined;
  const host = rest.slice(0, slash);
  const path = percentDecode(rest.slice(slash));
  if (path === undefined || CONTROL.test(path)) return undefined;
  return { host, path };
}

/** The text a percent-encoded path stands for, or nothing when it is not well encoded. */
function percentDecode(text: string): string | undefined {
  const bytes: number[] = [];
  const encoder = new TextEncoder();
  for (let at = 0; at < text.length;) {
    if (text[at] === "%") {
      const pair = text.slice(at + 1, at + 3);
      if (!/^[0-9a-fA-F]{2}$/.test(pair)) return undefined;
      bytes.push(parseInt(pair, 16));
      at += 3;
    } else {
      const point = text.codePointAt(at) ?? 0;
      const character = String.fromCodePoint(point);
      for (const byte of encoder.encode(character)) bytes.push(byte);
      at += character.length;
    }
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes));
  } catch {
    return undefined;
  }
}

/** Whether a report names this machine, compared as the core compares it. */
export function isThisMachine(reported: string, machine: Machine, windows: boolean): boolean {
  if (reported === "" || reported.toLowerCase() === "localhost") return true;
  const short = (name: string) => (name.split(".")[0] ?? "").toLowerCase();
  const own = short(machine.name ?? "");
  if (own === "") return false;
  const named = short(reported);
  if (named === own) return true;
  // Windows may hold only the beginning of a long name.
  return windows && [...own].length === NETBIOS_NAME_MAX_CHARS && named.startsWith(own);
}

/** A URL's path as a path of a Windows drive: `/C:/Users/me` is `C:\Users\me`. */
export function windowsPath(path: string): string | undefined {
  const match = /^\/([A-Za-z]):(\/.*)?$/.exec(path);
  if (!match) return undefined;
  const drive = (match[1] ?? "").toUpperCase();
  const below = match[2] ?? "";
  return below === "" ? `${drive}:\\` : `${drive}:${below.replaceAll("/", "\\")}`;
}

/** A path with the user's home folder at its start written as `~`. */
export function homeShortened(path: string, home: string | null, windows: boolean): string {
  if (!home) return path;
  const separator = windows ? "\\" : "/";
  const trimmed = home.length > 1 && home.endsWith(separator) ? home.slice(0, -1) : home;
  const same = windows
    ? (a: string, b: string) => a.toLowerCase() === b.toLowerCase()
    : (a: string, b: string) => a === b;
  if (same(path, trimmed)) return "~";
  const head = path.slice(0, trimmed.length + 1);
  if (same(head, trimmed + separator)) return `~${separator}${path.slice(trimmed.length + 1)}`;
  return path;
}

/** The last folder of a path, as a tab names where its pane is. */
export function lastFolder(path: string): string {
  if (path === "~" || path === "/") return path;
  const parts = path.split(/[\\/]/).filter((part) => part !== "");
  return parts.at(-1) ?? path;
}

/** Where a reported directory is, as it is read. */
export interface ReportedPlace {
  /** The machine the report names, when it is not this one. */
  readonly elsewhere: string | null;
  /** The path, as the machine it is on writes it. */
  readonly path: string;
}

/**
 * Where a report from a shell on this machine says it is. A report that names another
 * machine is read as a path of that machine, in the URL's own spelling.
 */
export function localPlace(
  url: string,
  machine: Machine,
  windows: boolean,
): ReportedPlace | undefined {
  const parsed = parseFileUrl(url);
  if (!parsed) return undefined;
  if (!isThisMachine(parsed.host, machine, windows)) {
    return { elsewhere: parsed.host, path: parsed.path };
  }
  const path = windows ? windowsPath(parsed.path) : parsed.path;
  return path === undefined ? undefined : { elsewhere: null, path };
}
