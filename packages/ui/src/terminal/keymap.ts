/**
 * The keys that are neither the shell's nor the program's.
 *
 * Which key runs which command is the configuration's to say, and everything else a
 * user types belongs to the program in the terminal. A few keys belong to neither: the
 * operating system or the webview acts on them, and a terminal that took them would
 * break something the user expects everywhere, such as closing a window.
 */

import type { KeyEventLike, Platform } from "./keyboard.ts";

/** Something a terminal does to itself in answer to a command. */
export type TerminalAction =
  | "copy"
  | "paste"
  | "scroll-page-up"
  | "scroll-page-down"
  | "scroll-top"
  | "scroll-bottom"
  | "clear"
  | "select-all";

/** What the decision depends on besides the key. */
export interface SystemKeyContext {
  readonly platform: Platform;
  /** Whether the key that opens the webview's developer tools is left alone. */
  readonly developerTools: boolean;
}

/**
 * Whether a key is left to the operating system or the webview whatever is bound to it:
 * closing the window and opening its menu, which stay with the window manager, and in
 * development the key that opens the developer tools.
 */
export function systemKey(event: KeyEventLike, context: SystemKeyContext): boolean {
  if (context.developerTools && event.key === "F12") return true;
  if (context.platform === "mac") return false;
  return event.altKey && !event.ctrlKey && (event.key === "F4" || event.code === "Space");
}

/**
 * Whether a key that no binding took is the system's rather than the program's: on
 * every platform, a key held with the Command or Windows key. Sent to a program, the
 * letter under it would be typed.
 */
export function unboundSystemKey(event: KeyEventLike): boolean {
  return event.metaKey;
}
