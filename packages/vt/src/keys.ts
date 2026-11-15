/**
 * Encoding key presses as the bytes a program expects.
 *
 * Which bytes a key sends depends on what the program has asked for: application cursor
 * keys, the Kitty keyboard protocol and its flags, how Alt is reported. The library holds
 * those rules, and the terminal holds the current settings, so the encoder is brought up
 * to date from the terminal before every key.
 *
 * Deciding what a DOM keyboard event means (which modifiers a layout consumed, when
 * AltGr is text, which shortcuts never reach the program) belongs to the caller. This
 * module starts from a key that has already been described.
 */

import type { Core, Vt } from "./module.ts";
import type { Terminal } from "./terminal.ts";

/**
 * Modifier bits, as `GhosttyMods` in ghostty/vt/key/event.h defines them. They are a
 * stable part of the library's interface and are not in its type manifest.
 */
export const Mod = {
  Shift: 1 << 0,
  Ctrl: 1 << 1,
  Alt: 1 << 2,
  Super: 1 << 3,
  CapsLock: 1 << 4,
  NumLock: 1 << 5,
  /** Set with `Shift` when the right-hand key is the one held. The same for the rest. */
  ShiftRight: 1 << 6,
  CtrlRight: 1 << 7,
  AltRight: 1 << 8,
  SuperRight: 1 << 9,
} as const;

/** What happened to the key. */
export type KeyAction = "press" | "release" | "repeat";

/** A key event to encode. */
export interface KeyInput {
  action: KeyAction;
  /**
   * The physical key, as the `code` of a DOM `KeyboardEvent` names it (`"KeyA"`,
   * `"ArrowUp"`, `"Numpad1"`). A code the library has no key for encodes as unidentified,
   * which still sends `text` if there is any.
   */
  code: string;
  /** `Mod` bits held during the event. */
  mods?: number;
  /** `Mod` bits the keyboard layout used up to produce `text`. */
  consumedMods?: number;
  /**
   * The text the key produced, before Ctrl or Alt changed it. Leave it out for keys
   * that produce control characters or no text.
   *
   * On a Mac, Option composes characters (Option+A is "å"). Treating Option as Alt is
   * the caller's job: pass the letter the key produces without Option. The library has
   * a setting for this, but it is compiled in only for macOS builds and the WebAssembly
   * build ignores it.
   */
  text?: string;
  /** The codepoint the key produces with no modifiers held, if known. */
  unshiftedCodepoint?: number;
  /** The event is part of an input-method composition and must not be sent as a key. */
  composing?: boolean;
}

/** Bytes the text buffer starts with; it grows to the longest text seen. */
const TEXT_MIN_BYTES = 64;

/**
 * The DOM `code` for a key of the library's key list.
 *
 * The library names its keys after the same standard the DOM uses, in upper case with
 * underscores, so the name converts mechanically: `ARROW_UP` is `ArrowUp`, `DIGIT_1` is
 * `Digit1`. Letters are the exception: the standard calls them `KeyA` to `KeyZ`.
 */
function domCode(name: string): string {
  if (/^[A-Z]$/.test(name)) return `Key${name}`;
  return name
    .split("_")
    .map((part) => part.charAt(0) + part.slice(1).toLowerCase())
    .join("");
}

/** Encodes key events for a terminal. One is enough for a window. */
export class KeyEncoder {
  readonly #core: Core;
  #encoder: number;
  #event: number;
  readonly #keys = new Map<string, number>();
  readonly #actions: Readonly<Record<KeyAction, number>>;
  // The library keeps the address of a key's text instead of copying it, so the text
  // lives in memory this encoder owns.
  #text = 0;
  #textCap = 0;

  constructor(vt: Vt) {
    const core = vt.core;
    this.#core = core;
    const m = core.manifest;

    for (const [name, value] of Object.entries(m.enumValues("GhosttyKey"))) {
      if (name !== "MAX_VALUE") this.#keys.set(domCode(name), value);
    }
    // Older engines report the Windows and Command keys under these names.
    this.#keys.set("OSLeft", m.enumValue("GhosttyKey", "META_LEFT"));
    this.#keys.set("OSRight", m.enumValue("GhosttyKey", "META_RIGHT"));

    this.#actions = {
      press: m.enumValue("GhosttyKeyAction", "PRESS"),
      release: m.enumValue("GhosttyKeyAction", "RELEASE"),
      repeat: m.enumValue("GhosttyKeyAction", "REPEAT"),
    };

    this.#encoder = core.newHandle("ghostty_key_encoder_new", (out) =>
      core.x.ghostty_key_encoder_new(0, out),
    );
    this.#event = 0;
    try {
      this.#event = core.newHandle("ghostty_key_event_new", (out) =>
        core.x.ghostty_key_event_new(0, out),
      );
    } catch (err) {
      this.free();
      throw err;
    }
  }

  /** Whether the library has a key for a DOM `code`. */
  knows(code: string): boolean {
    return this.#keys.has(code);
  }

  /**
   * The bytes to send for a key event under the terminal's current keyboard modes.
   * Empty when the event sends nothing, as most key releases do.
   */
  encode(terminal: Terminal, input: KeyInput): Uint8Array {
    if (this.#encoder === 0) throw new Error("this key encoder has been freed");
    const core = this.#core;
    const { x } = core;
    const event = this.#event;

    core.check(
      x.ghostty_key_encoder_set_from_terminal(this.#encoder, terminal.handle),
      "ghostty_key_encoder_set_from_terminal",
    );

    core.check(x.ghostty_key_event_set_action(event, this.#actions[input.action]), "set_action");
    core.check(x.ghostty_key_event_set_key(event, this.#keys.get(input.code) ?? 0), "set_key");
    core.check(x.ghostty_key_event_set_mods(event, input.mods ?? 0), "set_mods");
    core.check(
      x.ghostty_key_event_set_consumed_mods(event, input.consumedMods ?? 0),
      "set_consumed_mods",
    );
    core.check(x.ghostty_key_event_set_composing(event, input.composing ? 1 : 0), "set_composing");
    core.check(
      x.ghostty_key_event_set_unshifted_codepoint(event, input.unshiftedCodepoint ?? 0),
      "set_unshifted_codepoint",
    );
    this.#setText(input.text ?? "");

    try {
      return core.encode("ghostty_key_encoder_encode", (buf, cap, written) =>
        x.ghostty_key_encoder_encode(this.#encoder, event, buf, cap, written),
      );
    } finally {
      // Leave no address behind that the next change to the text buffer would invalidate.
      this.#setText("");
    }
  }

  /** Release the encoder. The object must not be used afterwards. */
  free(): void {
    const { x } = this.#core;
    if (this.#event !== 0) x.ghostty_key_event_free(this.#event);
    if (this.#encoder !== 0) x.ghostty_key_encoder_free(this.#encoder);
    if (this.#text !== 0) this.#core.free(this.#text, this.#textCap);
    this.#event = 0;
    this.#encoder = 0;
    this.#text = 0;
    this.#textCap = 0;
  }

  #setText(text: string): void {
    const core = this.#core;
    const m = core.manifest;
    let len = 0;
    if (text !== "") {
      // A UTF-16 code unit encodes to at most three bytes.
      const needed = text.length * 3;
      if (needed > this.#textCap) {
        if (this.#text !== 0) core.free(this.#text, this.#textCap);
        this.#text = 0;
        this.#textCap = 0;
        const cap = Math.max(needed, TEXT_MIN_BYTES);
        this.#text = core.alloc(cap);
        this.#textCap = cap;
      }
      const target = core.u8().subarray(this.#text, this.#text + needed);
      len = new TextEncoder().encodeInto(text, target).written;
    }
    // The string is passed by value in C; the export takes the address of a copy.
    const string = core.scratch(m.struct("GhosttyString").size);
    const view = core.view();
    view.setUint32(string + m.offset("GhosttyString", "ptr"), len === 0 ? 0 : this.#text, true);
    view.setUint32(string + m.offset("GhosttyString", "len"), len, true);
    core.check(core.x.ghostty_key_event_set_utf8(this.#event, string), "set_utf8");
  }
}
