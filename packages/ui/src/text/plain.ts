/**
 * Text from outside, made safe to show.
 *
 * What a program, a server or a file says of itself is shown beside the application's
 * own words, and among them in its questions. What it may hold is only what is seen: no
 * character that changes the order the text around it is read in, or that is there and
 * not seen, and no pile of marks that covers the lines above and below.
 *
 * The core makes what servers say safe by the same rule before it hands it on. Both are
 * held to `tests/text/plain.json`.
 */

import { TEXT_MAX_BYTES } from "../limits.ts";

/** The most marks and joiners in a run. */
const MARKS_MAX = 4;

/** Characters, by number: each one, or the first and the last of some that follow on. */
type Characters = readonly (readonly [first: number, last: number])[];

/** A line's end, or what lies between words: the words on either side are not one word. */
const SPACES: Characters = [
  [0x09, 0x0d],
  [0x85, 0x85],
  [0x2028, 0x2029],
];

/**
 * Control characters, those that change the direction text is laid out in, and those
 * that take part in text without being seen. Those that scripts and emoji are joined with
 * are not among them.
 */
const DROPPED: Characters = [
  [0x00, 0x1f],
  [0x7f, 0x9f],
  [0xad, 0xad],
  [0x34f, 0x34f],
  [0x61c, 0x61c],
  // Fillers that stand for a letter and show nothing.
  [0x115f, 0x1160],
  [0x17b4, 0x17b5],
  [0x180b, 0x180f],
  [0x200b, 0x200b],
  [0x200e, 0x200f],
  [0x202a, 0x202e],
  [0x2060, 0x206f],
  [0x3164, 0x3164],
  [0xfeff, 0xfeff],
  [0xffa0, 0xffa0],
  [0xfff0, 0xfffb],
  // Musical beams, tags and the selectors of further variants.
  [0x1d173, 0x1d17a],
  [0xe0000, 0xe0fff],
];

/**
 * What is nothing by itself: marks on the character before, of those that pile up, and
 * what joins or varies the characters beside it.
 */
const PILED: Characters = [
  [0x300, 0x36f],
  [0x483, 0x489],
  [0xe31, 0xe31],
  [0xe34, 0xe3a],
  [0xe47, 0xe4e],
  [0xeb1, 0xeb1],
  [0xeb4, 0xebc],
  [0xec8, 0xece],
  [0x1ab0, 0x1aff],
  [0x1dc0, 0x1dff],
  [0x200c, 0x200d],
  [0x20d0, 0x20ff],
  [0xfe00, 0xfe0f],
  [0xfe20, 0xfe2f],
];

const BLANK = /^\p{White_Space}$/u;

/** A text in which every character is one that stays as it is. */
const ASCII = /^[\x20-\x7e]*$/;

function among(characters: Characters, point: number): boolean {
  return characters.some(([first, last]) => point >= first && point <= last);
}

/** How many bytes a character takes as UTF-8. */
function bytesOf(point: number): number {
  return point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
}

/**
 * A text as it may be shown: one line, at most `max` bytes, with nothing in it that is
 * not seen. A text of which nothing would be seen is empty.
 */
export function plain(text: string, max: number = TEXT_MAX_BYTES): string {
  // Most text is of this kind, and it comes many times a second.
  if (text.length <= max && ASCII.test(text)) return /[^ ]/.test(text) ? text : "";
  let out = "";
  let bytes = 0;
  let piled = 0;
  let seen = false;
  for (const given of text) {
    const character = among(SPACES, given.codePointAt(0) ?? 0) ? " " : given;
    const point = character.codePointAt(0) ?? 0;
    if (among(DROPPED, point)) continue;
    const alone = !among(PILED, point);
    piled = alone ? 0 : piled + 1;
    if (piled > MARKS_MAX) continue;
    bytes += bytesOf(point);
    if (bytes > max) break;
    seen ||= alone && !BLANK.test(character);
    out += character;
  }
  return seen ? out : "";
}
