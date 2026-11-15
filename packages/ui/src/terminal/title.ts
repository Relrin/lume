/**
 * Making a title a program set safe to show.
 *
 * A title comes from output, so it is whatever a file or a remote machine wanted it to
 * be. Control characters are removed, since some of them reorder or hide the text that
 * follows, and the length is capped.
 */

import { TITLE_MAX_BYTES } from "../limits.ts";

/**
 * Control characters, and the characters that change the direction text is laid out in:
 * a title that uses those can show something other than what it says.
 */
const UNSAFE = /[\p{Cc}؜‎‏‪-‮⁦-⁩]/gu;

/** The title to show for what a program asked for. */
export function sanitizeTitle(title: string): string {
  const clean = title.replace(UNSAFE, "");
  const encoder = new TextEncoder();
  if (encoder.encode(clean).length <= TITLE_MAX_BYTES) return clean;

  // Cut on a character, never inside one.
  let bytes = 0;
  let kept = "";
  for (const character of clean) {
    bytes += encoder.encode(character).length;
    if (bytes > TITLE_MAX_BYTES) break;
    kept += character;
  }
  return kept;
}
