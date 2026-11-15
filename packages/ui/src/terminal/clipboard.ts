/**
 * The rules for what goes to and from the clipboard.
 *
 * Pasting is the one way text from somewhere else is typed into a program, and a paste
 * with a line break in it runs a command. A program writing to the clipboard is the one
 * way output leaves the terminal unasked. Both are allowed, within limits, and the user
 * is told.
 */

import type { ClipboardLocation } from "@lume/vt";

import { PASTE_MAX_BYTES, PASTE_PREVIEW_CHARS } from "../limits.ts";

/** The system clipboard, reached through the core. */
export interface ClipboardService {
  /** The text on the clipboard, or `null` when it holds none. */
  read(): Promise<string | null>;
  write(text: string): Promise<void>;
}

/** What to do with text about to be pasted. */
export type PasteDecision = "send" | "confirm" | "too-large";

/** The sequence that ends a bracketed paste. Text carrying it could break out of one. */
const BRACKET_END = "\x1b[201~";

/**
 * What to do with a paste.
 *
 * `safe` is the engine's own verdict, which is false for any text with a line break or
 * the bracket-ending sequence. A line break is harmless when the program has asked for
 * pastes to be bracketed, because it then sees the text as a paste and not as typing;
 * so with `bracketed` only the bracket-ending sequence still needs the user's say.
 */
export function pasteDecision(text: string, safe: boolean, bracketed: boolean): PasteDecision {
  if (new TextEncoder().encode(text).length > PASTE_MAX_BYTES) return "too-large";
  if (safe) return "send";
  if (bracketed && !text.includes(BRACKET_END)) return "send";
  return "confirm";
}

/** What the user is shown of a paste they are asked to confirm. */
export interface PastePreview {
  /** The start of the text, with control characters made visible. */
  text: string;
  /** Whether the text goes on beyond what is shown. */
  truncated: boolean;
  /** How many lines the whole text has. */
  lines: number;
}

/** Unicode has a visible symbol for each of the first 32 control characters, in order. */
const CONTROL_PICTURES = 0x2400;
/** The symbol for the delete character. */
const DELETE_PICTURE = "␡";
/** Shown for the other control characters, which have no symbol of their own. */
const REPLACEMENT = "�";

/** The preview of a paste. */
export function pastePreview(text: string): PastePreview {
  const lines = text.split(/\r\n|\r|\n/).length;
  const truncated = text.length > PASTE_PREVIEW_CHARS;
  // Control characters other than line breaks and tabs would be invisible; each is shown
  // as a picture of itself so the user sees that it is there.
  const shown = (truncated ? text.slice(0, PASTE_PREVIEW_CHARS) : text).replace(
    /\p{Cc}/gu,
    (control) => {
      if (control === "\n" || control === "\r" || control === "\t") return control;
      const code = control.charCodeAt(0);
      if (code < 0x20) return String.fromCodePoint(CONTROL_PICTURES + code);
      return code === 0x7f ? DELETE_PICTURE : REPLACEMENT;
    },
  );
  return { text: shown, truncated, lines };
}

/**
 * What a session's program may do to the clipboard, and whether the user has been told.
 * One per session: the notice is shown the first time each session uses the clipboard.
 */
export class ProgramClipboard {
  #noticed = false;

  /**
   * Whether a write is let through. Only the ordinary clipboard is written: the others
   * a program can name exist on some platforms only, and there they are what the user
   * selected by hand.
   */
  allows(location: ClipboardLocation): boolean {
    return location === "standard";
  }

  /** Whether the user is still to be told. True once, the first time it is asked. */
  noticeDue(): boolean {
    if (this.#noticed) return false;
    this.#noticed = true;
    return true;
  }
}
