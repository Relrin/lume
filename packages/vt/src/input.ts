/**
 * Focus reports and pasted text.
 */

import type { Vt } from "./module.ts";
import { Mode, type Terminal } from "./terminal.ts";

/**
 * The bytes that tell a program the terminal gained or lost focus. Send them only while
 * the program has focus reporting on (`terminal.mode(Mode.FocusEvent)`).
 */
export function encodeFocus(vt: Vt, focused: boolean): Uint8Array {
  const core = vt.core;
  const event = core.manifest.enumValue("GhosttyFocusEvent", focused ? "GAINED" : "LOST");
  return core.encode("ghostty_focus_encode", (buf, cap, written) =>
    core.x.ghostty_focus_encode(event, buf, cap, written),
  );
}

/**
 * Whether text can be pasted without asking the user first.
 *
 * Text is unsafe when it contains a line break, which a shell would run, or the sequence
 * that ends a bracketed paste, which would let the rest of the text escape the brackets.
 * The check is strict on purpose and does not look at the terminal's state.
 */
export function isPasteSafe(vt: Vt, text: string): boolean {
  const core = vt.core;
  const { ptr, len } = core.stageText(text);
  return core.x.ghostty_paste_is_safe(ptr, len) !== 0;
}

/**
 * The bytes to send for pasted text.
 *
 * Control characters that could act on the terminal are replaced with spaces. When the
 * program has bracketed paste on, the text is wrapped in its markers; otherwise line
 * breaks become carriage returns, as if the text had been typed.
 *
 * This does not decide whether the paste is safe: check `isPasteSafe` and ask the user
 * before calling it with text that is not.
 */
export function encodePaste(terminal: Terminal, text: string): Uint8Array {
  const core = terminal.core;
  const bracketed = terminal.mode(Mode.BracketedPaste);
  // The encoder rewrites the text where it stands, in the staging buffer.
  const { ptr, len } = core.stageText(text);
  return core.encode("ghostty_paste_encode", (buf, cap, written) =>
    core.x.ghostty_paste_encode(ptr, len, bracketed ? 1 : 0, buf, cap, written),
  );
}
