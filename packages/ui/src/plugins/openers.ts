/**
 * Which of the plugins' views offer to open a link.
 *
 * A plugin may say that one of its views opens files whose names fit a pattern, or
 * addresses of a scheme. The page looks only to decide whom to ask: a link that some
 * view offers to open goes to the application's command for opening things, which
 * matches it again for itself and has the user choose where there is a choice; any
 * other link is opened as links always were.
 */

import type { OpenerDef } from "../core/types.ts";

/**
 * Whether a file's name fits a pattern: the pattern's letters as they are, whatever
 * their case, with `*` for any run of characters.
 */
export function fits(pattern: string, name: string): boolean {
  const wanted = [...pattern.toLowerCase()];
  const given = [...name.toLowerCase()];
  let p = 0;
  let n = 0;
  // Where to go back to after the last `*`.
  let star: { p: number; n: number } | null = null;
  while (n < given.length) {
    if (p < wanted.length && wanted[p] === "*") {
      star = { p, n };
      p += 1;
    } else if (p < wanted.length && wanted[p] === given[n]) {
      p += 1;
      n += 1;
    } else if (star !== null) {
      star = { p: star.p, n: star.n + 1 };
      p = star.p + 1;
      n = star.n;
    } else {
      return false;
    }
  }
  return wanted.slice(p).every((c) => c === "*");
}

/** The name of the file a `file` address leads to, or nothing if it has none. */
function fileName(url: URL): string | null {
  const last = url.pathname.split("/").pop() ?? "";
  if (last === "") return null;
  try {
    return decodeURIComponent(last);
  } catch {
    return last;
  }
}

/** The views that offer to open `link`, in the order the plugins give them. */
export function openersFor(link: string, openers: readonly OpenerDef[]): readonly OpenerDef[] {
  let url: URL;
  try {
    url = new URL(link);
  } catch {
    return [];
  }
  const scheme = url.protocol.replace(/:$/, "").toLowerCase();
  if (scheme === "file") {
    const name = fileName(url);
    if (name === null) return [];
    return openers.filter((opener) => opener.patterns.some((pattern) => fits(pattern, name)));
  }
  return openers.filter((opener) => opener.schemes.some((one) => one.toLowerCase() === scheme));
}
