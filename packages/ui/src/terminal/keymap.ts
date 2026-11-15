/**
 * The few keys the shell keeps for itself.
 *
 * Everything a user types belongs to the program in the terminal, with a short list of
 * exceptions: copy and paste, the font size, scrolling back, and clearing. The list is
 * small on purpose, because every key on it is a key no program can receive.
 */

import type { KeyEventLike, Platform } from "./keyboard.ts";

/** Something the shell does itself in answer to a key. */
export type AppAction =
  | "copy"
  | "paste"
  | "font-larger"
  | "font-smaller"
  | "font-reset"
  | "scroll-page-up"
  | "scroll-page-down"
  | "scroll-top"
  | "scroll-bottom"
  | "clear"
  | "select-all";

/**
 * What a key is for: one of the shell's actions, `"system"` for a key that is left
 * entirely to the operating system or the webview, or `null` for a key that goes to the
 * program.
 */
export type KeyPurpose = AppAction | "system" | null;

/** What the decision depends on besides the key. */
export interface KeymapContext {
  readonly platform: Platform;
  /** Whether a full-screen program has the screen. Scrolling keys are then its own. */
  readonly alternateScreen: boolean;
  /** Whether the key that opens the webview's developer tools is left alone. */
  readonly developerTools: boolean;
}

/**
 * Whether the event is the key for an ASCII letter. Shortcuts follow the letter a key
 * types, so they move with the layout; on a layout that types no such letter, the key in
 * the letter's usual place is used.
 */
function letter(event: KeyEventLike, lower: string): boolean {
  const key = event.key.toLowerCase();
  if (key === lower) return true;
  const typesAscii = key.length === 1 && key.charCodeAt(0) < 0x80;
  return !typesAscii && event.code === `Key${lower.toUpperCase()}`;
}

const isEqual = (event: KeyEventLike) => event.code === "Equal" || event.code === "NumpadAdd";
const isMinus = (event: KeyEventLike) => event.code === "Minus" || event.code === "NumpadSubtract";
const isZero = (event: KeyEventLike) => event.code === "Digit0" || event.code === "Numpad0";

function scrolling(event: KeyEventLike, context: KeymapContext): AppAction | null {
  if (context.alternateScreen) return null;
  switch (event.key) {
    case "PageUp":
      return "scroll-page-up";
    case "PageDown":
      return "scroll-page-down";
    case "Home":
      return "scroll-top";
    case "End":
      return "scroll-bottom";
    default:
      return null;
  }
}

function macPurpose(event: KeyEventLike, context: KeymapContext): KeyPurpose {
  const { metaKey: cmd, ctrlKey: ctrl, altKey: option, shiftKey: shift } = event;
  if (cmd && !ctrl && !option) {
    if (!shift && letter(event, "c")) return "copy";
    if (!shift && letter(event, "v")) return "paste";
    if (!shift && letter(event, "k")) return "clear";
    if (!shift && letter(event, "a")) return "select-all";
    if (isEqual(event)) return "font-larger";
    if (!shift && isMinus(event)) return "font-smaller";
    if (!shift && isZero(event)) return "font-reset";
    const scroll = scrolling(event, context);
    if (scroll) return scroll;
  }
  // Every other Command key belongs to the system: quit, hide, close, switch window.
  // Sent to a program, the letter under it would be typed.
  if (cmd) return "system";
  if (shift && !ctrl && !option) return scrolling(event, context);
  return null;
}

function otherPurpose(event: KeyEventLike, context: KeymapContext): KeyPurpose {
  const { ctrlKey: ctrl, altKey: alt, shiftKey: shift, metaKey: meta } = event;
  // Closing the window and opening its menu stay with the window manager.
  if (alt && !ctrl && (event.key === "F4" || event.code === "Space")) return "system";
  if (meta) return "system";
  if (ctrl && shift && !alt) {
    if (letter(event, "c")) return "copy";
    if (letter(event, "v")) return "paste";
    if (letter(event, "k")) return "clear";
  }
  if (event.key === "Insert" && !alt) {
    if (ctrl && !shift) return "copy";
    if (shift && !ctrl) return "paste";
  }
  if (ctrl && !alt) {
    if (isEqual(event)) return "font-larger";
    if (!shift && isMinus(event)) return "font-smaller";
    if (!shift && isZero(event)) return "font-reset";
  }
  if (shift && !ctrl && !alt) return scrolling(event, context);
  return null;
}

/** What a key is for. */
export function keyPurpose(event: KeyEventLike, context: KeymapContext): KeyPurpose {
  if (context.developerTools && event.key === "F12") return "system";
  return context.platform === "mac" ? macPurpose(event, context) : otherPurpose(event, context);
}
