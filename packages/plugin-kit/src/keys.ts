/**
 * Keys a view passes on to the window.
 *
 * While the keyboard is in a view, the window sees none of its keys: the view is another
 * document. So that the keys that move about the window still work from inside it, a
 * view passes on the keys it has no use for. The window runs only a fixed few commands
 * from them (another pane, another tab, the palette, a side panel, the bar), whatever is
 * bound.
 */

/** A key as the window is told of it. */
export interface PassedKey {
  readonly key: string;
  readonly code: string;
  readonly mods: readonly ("ctrl" | "alt" | "shift" | "meta")[];
}

/** The part of a keyboard event the rule reads. */
export interface KeyLike {
  readonly key: string;
  readonly code: string;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly shiftKey: boolean;
  readonly metaKey: boolean;
  readonly defaultPrevented?: boolean;
  readonly isComposing?: boolean;
}

/** A key as it is passed on. */
export function describeKey(event: KeyLike): PassedKey {
  const mods: ("ctrl" | "alt" | "shift" | "meta")[] = [];
  if (event.ctrlKey) mods.push("ctrl");
  if (event.altKey) mods.push("alt");
  if (event.shiftKey) mods.push("shift");
  if (event.metaKey) mods.push("meta");
  return { key: event.key, code: event.code, mods };
}

/**
 * Whether the window has any use for a key: one pressed with Ctrl, Alt or the logo key,
 * or a function key. Plain typing stays in the view.
 */
export function worthForwarding(key: PassedKey): boolean {
  if (key.key.length === 0 || key.key.length > 128) return false;
  if (key.mods.includes("ctrl") || key.mods.includes("alt") || key.mods.includes("meta")) {
    // A modifier pressed alone is no key yet.
    return !["Control", "Alt", "Meta", "Shift", "AltGraph"].includes(key.key);
  }
  return /^F([1-9]|1[0-9]|2[0-4])$/.test(key.key);
}

/** The keys a field uses itself when held with Ctrl or the logo key. */
const EDITING = new Set([
  "a",
  "c",
  "v",
  "x",
  "z",
  "y",
  "arrowleft",
  "arrowright",
  "arrowup",
  "arrowdown",
  "backspace",
  "delete",
  "home",
  "end",
]);

/**
 * Whether a key is a field's own while the keyboard is in one: selecting, copying,
 * pasting, undoing, and moving by words. Such a key is not passed on from a field, so
 * that it does not also move about the window.
 */
export function editingKey(key: PassedKey): boolean {
  if (key.mods.includes("alt")) return false;
  if (!key.mods.includes("ctrl") && !key.mods.includes("meta")) return false;
  return EDITING.has(key.key.toLowerCase());
}

/**
 * The key to pass on for a keyboard event, or none: the view used it itself, it is
 * plain typing, or it is a field's own.
 */
export function keyToPass(event: KeyLike, inField: boolean): PassedKey | null {
  if (event.defaultPrevented === true || event.isComposing === true) return null;
  const key = describeKey(event);
  if (!worthForwarding(key)) return null;
  if (inField && editingKey(key)) return null;
  return key;
}
