/**
 * A key event as the chord the configuration writes for it.
 *
 * A binding names its keys as text: modifiers and one key joined by `+`, as in
 * `ctrl+shift+t`. The key is the character the key types without Shift, or a name such
 * as `enter` or `pageup`, and Shift is always written out. The core sends every
 * binding in one canonical spelling; this makes the same spelling from an event, so
 * the two compare as text.
 *
 * Nothing here touches the page beyond reading the event, so the rules are tested with
 * plain objects.
 */

import {
  isModifierKey,
  KeyLayout,
  type KeyEventLike,
  type Platform,
} from "../terminal/keyboard.ts";

/** How events are read. */
export interface ChordMapping {
  readonly platform: Platform;
  /** What each key types with no modifier held, on the layout in use. */
  readonly layout: KeyLayout;
}

/** The parts of a chord. */
export interface ChordParts {
  readonly ctrl: boolean;
  readonly alt: boolean;
  readonly shift: boolean;
  /** The Command key of a Mac, the Windows key, the Super key. */
  readonly logo: boolean;
  /** A key's name, or the one character it types without Shift, in lower case. */
  readonly key: string;
}

/** The keys that have names, by what a browser calls them. */
const NAMED_KEYS: ReadonlyMap<string, string> = new Map([
  ["Enter", "enter"],
  ["Tab", "tab"],
  [" ", "space"],
  ["Spacebar", "space"],
  ["Escape", "escape"],
  ["Esc", "escape"],
  ["Backspace", "backspace"],
  ["Delete", "delete"],
  ["Del", "delete"],
  ["Insert", "insert"],
  ["Home", "home"],
  ["End", "end"],
  ["PageUp", "pageup"],
  ["PageDown", "pagedown"],
  ["ArrowUp", "up"],
  ["ArrowDown", "down"],
  ["ArrowLeft", "left"],
  ["ArrowRight", "right"],
  ["Up", "up"],
  ["Down", "down"],
  ["Left", "left"],
  ["Right", "right"],
  ["ContextMenu", "contextmenu"],
]);

/** The highest function key there is a name for. */
const FUNCTION_KEY_MAX = 24;

const FUNCTION_KEY = /^F([1-9][0-9]?)$/;

/** A keyboard nobody has said anything about: every key types what it does on a US one. */
const US = new KeyLayout();

/** One character that prints, as opposed to a key's name. */
function printable(key: string): boolean {
  const first = key.codePointAt(0);
  if (first === undefined) return false;
  if (key.length !== (first > 0xffff ? 2 : 1)) return false;
  return first > 0x20 && first !== 0x7f && !(first >= 0x80 && first <= 0x9f);
}

function isAscii(character: string): boolean {
  return character.length === 1 && character.charCodeAt(0) < 0x80;
}

function isAsciiLetter(character: string): boolean {
  return character.length === 1 && character >= "a" && character <= "z";
}

/**
 * Whether the event is a character typed with AltGr rather than a key pressed with
 * Ctrl and Alt. AltGr types the third character of a key, and on Windows it arrives
 * looking like Ctrl and Alt together, sometimes without the browser saying which it
 * is; a character that is not the key's own gives it away.
 */
function typedWithAltGr(event: KeyEventLike, mapping: ChordMapping): boolean {
  if (event.getModifierState("AltGraph")) return true;
  if (mapping.platform !== "windows" || !event.ctrlKey || !event.altKey) return false;
  if (!printable(event.key)) return false;
  const base = mapping.layout.base(event.code);
  return base !== undefined && event.key.toLowerCase() !== base.toLowerCase();
}

/** The name of the key an event is for, or nothing when it has none a binding could use. */
function keyName(event: KeyEventLike, mapping: ChordMapping): string | undefined {
  const named = NAMED_KEYS.get(event.key);
  if (named !== undefined) return named;
  const functionKey = FUNCTION_KEY.exec(event.key);
  if (functionKey) {
    return Number(functionKey[1]) <= FUNCTION_KEY_MAX ? event.key.toLowerCase() : undefined;
  }

  const typed = printable(event.key) ? event.key.toLowerCase() : undefined;
  // A letter is the key that types it, wherever the layout puts it.
  if (typed !== undefined && isAsciiLetter(typed)) return typed;
  // Without Shift, and without Option making something else of it, what was typed is
  // the key's own character.
  const plain = !event.shiftKey && !(mapping.platform === "mac" && event.altKey);
  if (typed !== undefined && isAscii(typed) && plain) return typed;

  // Otherwise the key is named by what it types without Shift on this layout, and on
  // a layout that types nothing from ASCII there, by what it types on a US keyboard:
  // Ctrl+Shift+C is the same key on a Cyrillic layout as on a Latin one.
  const base = mapping.layout.base(event.code)?.toLowerCase();
  if (base !== undefined && printable(base) && isAscii(base)) return base;
  const us = US.base(event.code);
  if (us !== undefined && printable(us)) return us;
  if (base !== undefined && printable(base)) return base;
  return typed;
}

/** A chord in its one canonical spelling. */
export function formatChord(chord: ChordParts): string {
  let written = "";
  if (chord.ctrl) written += "ctrl+";
  if (chord.alt) written += "alt+";
  if (chord.shift) written += "shift+";
  if (chord.logo) written += "super+";
  return written + chord.key;
}

/**
 * The chord a `keydown` stands for, spelt as the configuration spells it, or `null`
 * when the event is not a key a binding could name: a modifier on its own, a key that
 * is part of typing through an input method or a dead key, or a character typed with
 * AltGr. The last matters on keyboards that type `{`, `[` or `@` with AltGr, which
 * must reach the program even where Ctrl+Alt and the same key is bound.
 */
export function chordOf(event: KeyEventLike, mapping: ChordMapping): string | null {
  if (
    event.isComposing ||
    event.keyCode === 229 ||
    event.key === "Process" ||
    event.key === "Dead" ||
    event.key === "Unidentified"
  ) {
    return null;
  }
  if (isModifierKey(event)) return null;
  if (typedWithAltGr(event, mapping)) return null;
  const key = keyName(event, mapping);
  if (key === undefined) return null;
  return formatChord({
    ctrl: event.ctrlKey,
    alt: event.altKey,
    shift: event.shiftKey,
    logo: event.metaKey,
    key,
  });
}
