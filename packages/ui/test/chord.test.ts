import { describe, expect, it } from "vitest";

import { chordOf, formatChord, type ChordMapping } from "../src/keys/chord.ts";
import { KeyLayout, type Platform } from "../src/terminal/keyboard.ts";
import { keyEvent, type KeyOptions } from "./keys.ts";

/** What a few keys type without a modifier on a German keyboard. */
const GERMAN = new Map([
  ["KeyY", "z"],
  ["KeyZ", "y"],
  ["Digit7", "7"],
  ["Digit8", "8"],
  ["KeyQ", "q"],
  ["Minus", "ß"],
  ["Semicolon", "ö"],
  ["BracketRight", "+"],
  ["Slash", "-"],
]);

/** And on a Russian one. */
const RUSSIAN = new Map([
  ["KeyC", "с"],
  ["KeyT", "е"],
  ["KeyV", "м"],
  ["Digit1", "1"],
  ["Comma", "б"],
]);

function mapping(
  platform: Platform = "windows",
  known?: ReadonlyMap<string, string>,
): ChordMapping {
  return { platform, layout: new KeyLayout(known) };
}

function chord(
  key: string,
  code: string,
  options: KeyOptions = {},
  on: ChordMapping = mapping(),
): string | null {
  return chordOf(keyEvent(key, code, options), on);
}

const CTRL = { ctrl: true };
const CTRL_SHIFT = { ctrl: true, shift: true };
const CTRL_ALT = { ctrl: true, alt: true };

describe("a chord in its canonical spelling", () => {
  it("writes the modifiers in one order, then the key", () => {
    const all = { ctrl: true, alt: true, shift: true, logo: true, key: "t" };
    expect(formatChord(all)).toBe("ctrl+alt+shift+super+t");
    expect(formatChord({ ...all, ctrl: false, shift: false })).toBe("alt+super+t");
    expect(formatChord({ ctrl: false, alt: false, shift: false, logo: false, key: "f5" })).toBe(
      "f5",
    );
    expect(formatChord({ ...all, alt: false, shift: false, logo: false, key: "+" })).toBe("ctrl++");
  });
});

describe("keys on a US keyboard", () => {
  it("names a letter by the letter, with Shift written out", () => {
    expect(chord("t", "KeyT", CTRL)).toBe("ctrl+t");
    expect(chord("T", "KeyT", CTRL_SHIFT)).toBe("ctrl+shift+t");
    expect(chord("T", "KeyT", { ...CTRL_SHIFT, alt: true })).toBe("ctrl+alt+shift+t");
    expect(chord("a", "KeyA")).toBe("a");
    // Caps Lock changes what is typed, not which key it is.
    expect(chord("A", "KeyA", { ctrl: true, states: ["CapsLock"] })).toBe("ctrl+a");
  });

  it("names a key that types a sign by what it types without Shift", () => {
    expect(chord("=", "Equal", CTRL)).toBe("ctrl+=");
    expect(chord("+", "Equal", CTRL_SHIFT)).toBe("ctrl+shift+=");
    expect(chord("-", "Minus", CTRL)).toBe("ctrl+-");
    expect(chord("_", "Minus", CTRL_SHIFT)).toBe("ctrl+shift+-");
    expect(chord("<", "Comma", CTRL_SHIFT)).toBe("ctrl+shift+,");
    expect(chord("0", "Digit0", CTRL)).toBe("ctrl+0");
    expect(chord("!", "Digit1", CTRL_SHIFT)).toBe("ctrl+shift+1");
    expect(chord("{", "BracketLeft", { meta: true, shift: true }, mapping("mac"))).toBe(
      "shift+super+[",
    );
  });

  it("names the keys that have names", () => {
    const named: [string, string, string][] = [
      ["Enter", "Enter", "enter"],
      ["Tab", "Tab", "tab"],
      [" ", "Space", "space"],
      ["Escape", "Escape", "escape"],
      ["Backspace", "Backspace", "backspace"],
      ["Delete", "Delete", "delete"],
      ["Insert", "Insert", "insert"],
      ["Home", "Home", "home"],
      ["End", "End", "end"],
      ["PageUp", "PageUp", "pageup"],
      ["PageDown", "PageDown", "pagedown"],
      ["ArrowUp", "ArrowUp", "up"],
      ["ArrowDown", "ArrowDown", "down"],
      ["ArrowLeft", "ArrowLeft", "left"],
      ["ArrowRight", "ArrowRight", "right"],
      ["ContextMenu", "ContextMenu", "contextmenu"],
    ];
    for (const [key, code, name] of named) {
      expect(chord(key, code, CTRL_SHIFT)).toBe(`ctrl+shift+${name}`);
    }
    expect(chord("Tab", "Tab", CTRL)).toBe("ctrl+tab");
    expect(chord("Insert", "Insert", { shift: true })).toBe("shift+insert");
    expect(chord("PageUp", "PageUp", { shift: true })).toBe("shift+pageup");
  });

  it("names the function keys there are names for", () => {
    expect(chord("F1", "F1")).toBe("f1");
    expect(chord("F5", "F5", CTRL)).toBe("ctrl+f5");
    expect(chord("F12", "F12", { shift: true })).toBe("shift+f12");
    expect(chord("F24", "F24")).toBe("f24");
    expect(chord("F25", "F25")).toBeNull();
  });

  it("names the keypad's keys by what they do", () => {
    expect(chord("1", "Numpad1", { ctrl: true, states: ["NumLock"] })).toBe("ctrl+1");
    expect(chord("0", "Numpad0", { ctrl: true, states: ["NumLock"] })).toBe("ctrl+0");
    expect(chord("+", "NumpadAdd", CTRL)).toBe("ctrl++");
    expect(chord("-", "NumpadSubtract", CTRL)).toBe("ctrl+-");
    expect(chord("Enter", "NumpadEnter", CTRL_SHIFT)).toBe("ctrl+shift+enter");
    // With Num Lock off the keypad moves the cursor.
    expect(chord("PageUp", "Numpad9", { shift: true })).toBe("shift+pageup");
    expect(chord("ArrowLeft", "Numpad4", CTRL_SHIFT)).toBe("ctrl+shift+left");
  });
});

describe("what is not a chord", () => {
  it("is a modifier on its own", () => {
    expect(chord("Control", "ControlLeft", CTRL)).toBeNull();
    expect(chord("Shift", "ShiftLeft", CTRL_SHIFT)).toBeNull();
    expect(chord("Alt", "AltLeft", { alt: true })).toBeNull();
    expect(chord("Meta", "MetaLeft", { meta: true })).toBeNull();
    expect(chord("AltGraph", "AltRight", CTRL_ALT)).toBeNull();
    expect(chord("CapsLock", "CapsLock")).toBeNull();
  });

  it("is a key on its way through an input method or a dead key", () => {
    expect(chord("a", "KeyA", { composing: true })).toBeNull();
    expect(chord("Process", "KeyA", { keyCode: 229 })).toBeNull();
    expect(chord("a", "KeyA", { keyCode: 229 })).toBeNull();
    expect(chord("Dead", "BracketLeft", CTRL)).toBeNull();
    expect(chord("Unidentified", "")).toBeNull();
  });
});

describe("AltGr", () => {
  const german = mapping("windows", GERMAN);

  it("types its character on a German keyboard instead of being Ctrl+Alt", () => {
    // AltGr+7 types `{`, AltGr+8 `[`, AltGr+Q `@`.
    const altGr = { ...CTRL_ALT, states: ["AltGraph"] };
    expect(chord("{", "Digit7", altGr, german)).toBeNull();
    expect(chord("[", "Digit8", altGr, german)).toBeNull();
    expect(chord("@", "KeyQ", altGr, german)).toBeNull();
  });

  it("is recognised on Windows by its character when the browser does not name it", () => {
    expect(chord("{", "Digit7", CTRL_ALT, german)).toBeNull();
    expect(chord("@", "KeyQ", CTRL_ALT, german)).toBeNull();
    // The same holds while the layout is not known and a US keyboard is assumed.
    expect(chord("{", "Digit7", CTRL_ALT)).toBeNull();
    // A Polish keyboard types ą with AltGr+A.
    expect(chord("ą", "KeyA", CTRL_ALT)).toBeNull();
  });

  it("is told by its own state elsewhere, where it does not look like Ctrl+Alt", () => {
    const linux = mapping("linux", GERMAN);
    expect(chord("{", "Digit7", { states: ["AltGraph"] }, linux)).toBeNull();
    expect(chord("@", "KeyQ", { states: ["AltGraph"] }, linux)).toBeNull();
  });

  it("leaves Ctrl+Alt with a key's own character a chord", () => {
    expect(chord("1", "Digit1", CTRL_ALT)).toBe("ctrl+alt+1");
    expect(chord("7", "Digit7", CTRL_ALT, german)).toBe("ctrl+alt+7");
    expect(chord("a", "KeyA", CTRL_ALT)).toBe("ctrl+alt+a");
    expect(chord("ArrowLeft", "ArrowLeft", CTRL_ALT)).toBe("ctrl+alt+left");
    // Elsewhere Ctrl and Alt are two modifiers whatever the key typed.
    expect(chord("1", "Digit1", CTRL_ALT, mapping("linux"))).toBe("ctrl+alt+1");
  });
});

describe("keys on other layouts", () => {
  it("follows the letter a key types on a German keyboard", () => {
    const german = mapping("windows", GERMAN);
    expect(chord("Z", "KeyY", CTRL_SHIFT, german)).toBe("ctrl+shift+z");
    expect(chord("y", "KeyZ", CTRL, german)).toBe("ctrl+y");
    // The key beside Enter types `+` without Shift there.
    expect(chord("+", "BracketRight", CTRL, german)).toBe("ctrl++");
    expect(chord("-", "Slash", CTRL, german)).toBe("ctrl+-");
    expect(chord("_", "Slash", CTRL_SHIFT, german)).toBe("ctrl+shift+-");
  });

  it("uses the key in a letter's usual place where the layout types no such letter", () => {
    const russian = mapping("windows", RUSSIAN);
    expect(chord("С", "KeyC", CTRL_SHIFT, russian)).toBe("ctrl+shift+c");
    expect(chord("с", "KeyC", CTRL, russian)).toBe("ctrl+c");
    expect(chord("Е", "KeyT", CTRL_SHIFT, russian)).toBe("ctrl+shift+t");
    expect(chord("Б", "Comma", CTRL_SHIFT, russian)).toBe("ctrl+shift+,");
    expect(chord("1", "Digit1", CTRL_ALT, russian)).toBe("ctrl+alt+1");
    // The same before anything is known about the layout.
    expect(chord("С", "KeyC", CTRL_SHIFT)).toBe("ctrl+shift+c");
    expect(chord("ö", "Semicolon", CTRL, mapping("linux", GERMAN))).toBe("ctrl+;");
  });

  it("learns the layout from what was typed when the browser cannot say", () => {
    const layout = new KeyLayout();
    layout.learn("Slash", "-");
    const learnt: ChordMapping = { platform: "linux", layout };
    expect(chord("_", "Slash", CTRL_SHIFT, learnt)).toBe("ctrl+shift+-");
  });
});

describe("keys on a Mac", () => {
  const mac = mapping("mac");

  it("writes Command as the logo key", () => {
    expect(chord("t", "KeyT", { meta: true }, mac)).toBe("super+t");
    expect(chord("d", "KeyD", { meta: true, shift: true }, mac)).toBe("shift+super+d");
    expect(chord(",", "Comma", { meta: true }, mac)).toBe("super+,");
    expect(chord("1", "Digit1", { meta: true }, mac)).toBe("super+1");
    expect(chord("ArrowLeft", "ArrowLeft", { meta: true, alt: true }, mac)).toBe("alt+super+left");
    expect(chord("]", "BracketRight", { meta: true, shift: true }, mac)).toBe("shift+super+]");
  });

  it("names a key by its own character whatever Option made of it", () => {
    expect(chord("å", "KeyA", { alt: true }, mac)).toBe("alt+a");
    expect(chord("≠", "Equal", { alt: true, meta: true }, mac)).toBe("alt+super+=");
    expect(chord("@", "KeyL", { alt: true }, mapping("mac", new Map([["KeyL", "l"]])))).toBe(
      "alt+l",
    );
  });
});
