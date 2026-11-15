/**
 * Turning a DOM keyboard event into a key the terminal engine can encode.
 *
 * The engine knows which bytes a key sends under the modes a program has set. It does not
 * know what a browser means by its events: which modifiers the keyboard layout used up to
 * produce a character, that AltGr arrives looking like Ctrl+Alt, or what a key would have
 * typed without Shift. Those are settled here, and nothing here touches the DOM beyond
 * reading the event, so the rules are tested with plain objects.
 */

import { Mod, type KeyInput } from "@lume/vt";

/** The parts of a DOM `KeyboardEvent` the mapping reads. */
export interface KeyEventLike {
  readonly key: string;
  readonly code: string;
  readonly ctrlKey: boolean;
  readonly shiftKey: boolean;
  readonly altKey: boolean;
  readonly metaKey: boolean;
  readonly repeat: boolean;
  readonly isComposing: boolean;
  readonly keyCode: number;
  getModifierState(key: string): boolean;
}

/** The operating system's keyboard conventions differ; these are the three kinds. */
export type Platform = "mac" | "windows" | "linux";

/** The platform this page runs on, from what the browser says about itself. */
export function currentPlatform(): Platform {
  const hint = `${navigator.platform} ${navigator.userAgent}`.toLowerCase();
  if (hint.includes("mac")) return "mac";
  if (hint.includes("win")) return "windows";
  return "linux";
}

/** What each key types on a US keyboard with no modifier held. */
const US_LAYOUT: ReadonlyMap<string, string> = new Map<string, string>([
  ...Array.from({ length: 26 }, (_, i): [string, string] => [
    `Key${String.fromCharCode(65 + i)}`,
    String.fromCharCode(97 + i),
  ]),
  ...Array.from({ length: 10 }, (_, i): [string, string] => [`Digit${i}`, String(i)]),
  ["Minus", "-"],
  ["Equal", "="],
  ["BracketLeft", "["],
  ["BracketRight", "]"],
  ["Backslash", "\\"],
  ["Semicolon", ";"],
  ["Quote", "'"],
  ["Backquote", "`"],
  ["Comma", ","],
  ["Period", "."],
  ["Slash", "/"],
  ["Space", " "],
  ["IntlBackslash", "\\"],
]);

/**
 * What each key types with no modifier held, on the keyboard layout in use.
 *
 * Some protocols name a key by that character, so Ctrl+C must be reported as "c" on every
 * layout. A browser may be able to say what the layout is. Where it cannot, each key
 * pressed without modifiers teaches its character, and until a key has been seen a US
 * keyboard is assumed.
 */
export class KeyLayout {
  readonly #known: ReadonlyMap<string, string>;
  readonly #learnt = new Map<string, string>();

  /** `known` is the layout as the browser reports it, by key code, if it can. */
  constructor(known: ReadonlyMap<string, string> = new Map()) {
    this.#known = known;
  }

  /** The character the key types with no modifier held, if there is one. */
  base(code: string): string | undefined {
    return this.#known.get(code) ?? this.#learnt.get(code) ?? US_LAYOUT.get(code);
  }

  /** Records what a key typed with no modifier held. */
  learn(code: string, character: string): void {
    this.#learnt.set(code, character);
  }
}

/** How events are read. */
export interface KeyMapping {
  readonly platform: Platform;
  /** Whether Option on a Mac acts as Alt instead of typing its own characters. */
  readonly macOptionAsAlt: boolean;
  readonly layout: KeyLayout;
  /** Whether the engine has a key of this name. Unknown names fall back to the event's. */
  knows?(code: string): boolean;
}

/**
 * The engine's names for the keypad keys when Num Lock is off and they move the cursor.
 * The DOM gives those the code of the digit and the key of the movement.
 */
const KEYPAD_NAVIGATION: ReadonlyMap<string, string> = new Map([
  ["ArrowUp", "NumpadUp"],
  ["ArrowDown", "NumpadDown"],
  ["ArrowLeft", "NumpadLeft"],
  ["ArrowRight", "NumpadRight"],
  ["Home", "NumpadHome"],
  ["End", "NumpadEnd"],
  ["PageUp", "NumpadPageUp"],
  ["PageDown", "NumpadPageDown"],
  ["Insert", "NumpadInsert"],
  ["Delete", "NumpadDelete"],
  ["Clear", "NumpadBegin"],
]);

const MODIFIER_KEYS: ReadonlySet<string> = new Set([
  "Shift",
  "Control",
  "Alt",
  "AltGraph",
  "Meta",
  "OS",
  "CapsLock",
  "NumLock",
  "ScrollLock",
  "Fn",
  "FnLock",
  "Hyper",
  "Super",
  "Symbol",
  "SymbolLock",
]);

/** Whether the event is a modifier key on its own. */
export function isModifierKey(event: KeyEventLike): boolean {
  return MODIFIER_KEYS.has(event.key);
}

/** Whether `key` is one character that prints, as opposed to a key's name. */
function printable(key: string): boolean {
  const first = key.codePointAt(0);
  if (first === undefined) return false;
  // One code point, which may be two UTF-16 units.
  if (key.length !== (first > 0xffff ? 2 : 1)) return false;
  return first >= 0x20 && first !== 0x7f && !(first >= 0x80 && first <= 0x9f);
}

/** The modifier bits an event carries. */
export function modifiers(event: KeyEventLike): number {
  let mods = 0;
  if (event.shiftKey) mods |= Mod.Shift;
  if (event.ctrlKey) mods |= Mod.Ctrl;
  if (event.altKey) mods |= Mod.Alt;
  if (event.metaKey) mods |= Mod.Super;
  if (event.getModifierState("CapsLock")) mods |= Mod.CapsLock;
  if (event.getModifierState("NumLock")) mods |= Mod.NumLock;
  return mods;
}

function keyCode(event: KeyEventLike, mapping: KeyMapping): string {
  if (event.code.startsWith("Numpad")) {
    const navigation = KEYPAD_NAVIGATION.get(event.key);
    if (navigation && (mapping.knows?.(navigation) ?? true)) return navigation;
  }
  return event.code;
}

/**
 * The key a `keydown` stands for, or `null` when the event is not a key to send.
 *
 * An event that is part of typing through an input method, or the first half of a
 * character made with a dead key, is not a key: the text it leads to arrives later
 * through the composition and input events.
 */
export function mapKey(event: KeyEventLike, mapping: KeyMapping): KeyInput | null {
  if (
    event.isComposing ||
    event.keyCode === 229 ||
    event.key === "Process" ||
    event.key === "Dead"
  ) {
    return null;
  }

  const { platform, layout } = mapping;
  let mods = modifiers(event);
  let text = printable(event.key) ? event.key : undefined;
  const base = layout.base(event.code);

  // AltGr types a character of its own. On Windows it arrives as Ctrl and Alt together,
  // sometimes without the browser saying it is AltGr; a character that is not the key's
  // own gives it away. Left in, those two modifiers would turn "@" into a control code.
  const altGr =
    text !== undefined &&
    (event.getModifierState("AltGraph") ||
      (platform === "windows" &&
        event.ctrlKey &&
        event.altKey &&
        base !== undefined &&
        text.toLowerCase() !== base.toLowerCase()));
  if (altGr) mods &= ~(Mod.Ctrl | Mod.Alt);

  if (platform === "mac" && event.altKey && !altGr) {
    if (mapping.macOptionAsAlt) {
      // The program should see Alt and the key's own letter, not what Option made of it.
      if (base !== undefined) text = event.shiftKey ? base.toUpperCase() : base;
    } else {
      // Option was used to type the character, as on any Mac; it is not also Alt.
      mods &= ~Mod.Alt;
    }
  }

  // Num Lock changes no character of the main keys, so it does not spoil the lesson.
  if (text !== undefined && (mods & ~Mod.NumLock) === 0) layout.learn(event.code, text);

  // Shift is used up when it changed what the key types. Whether it did is known when
  // the key's own character is: without that, any text but a space is taken to have
  // needed it.
  let consumedMods = 0;
  if (text !== undefined && event.shiftKey) {
    if (base === undefined ? text !== " " : text !== base) consumedMods |= Mod.Shift;
  }

  const input: KeyInput = {
    action: event.repeat ? "repeat" : "press",
    code: keyCode(event, mapping),
    mods,
    consumedMods,
  };
  if (text !== undefined) input.text = text;
  const unshifted = base ?? (text !== undefined && !event.shiftKey ? text : undefined);
  if (unshifted !== undefined) {
    const codepoint = unshifted.toLowerCase().codePointAt(0);
    if (codepoint !== undefined) input.unshiftedCodepoint = codepoint;
  }
  return input;
}

/** The key a `keyup` stands for. A release carries no text. */
export function mapKeyRelease(event: KeyEventLike, mapping: KeyMapping): KeyInput {
  const input: KeyInput = {
    action: "release",
    code: keyCode(event, mapping),
    mods: modifiers(event),
  };
  const codepoint = mapping.layout.base(event.code)?.toLowerCase().codePointAt(0);
  if (codepoint !== undefined) input.unshiftedCodepoint = codepoint;
  return input;
}

/** Text that did not come from a key: an input method's result, a dead-key character. */
export function textInput(text: string): KeyInput {
  // A key the engine has no name for sends its text as it is, under every mode.
  return { action: "press", code: "", text };
}
