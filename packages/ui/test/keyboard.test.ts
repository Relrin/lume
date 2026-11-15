import { Mod } from "@lume/vt";
import { describe, expect, it } from "vitest";

import {
  isModifierKey,
  KeyLayout,
  mapKey,
  mapKeyRelease,
  modifiers,
  textInput,
  type KeyMapping,
  type Platform,
} from "../src/terminal/keyboard.ts";
import { keyEvent } from "./keys.ts";

function mapping(platform: Platform = "windows", overrides: Partial<KeyMapping> = {}): KeyMapping {
  return { platform, macOptionAsAlt: false, layout: new KeyLayout(), ...overrides };
}

const code = (character: string) => character.codePointAt(0)!;

describe("reading a key event", () => {
  it("passes a plain letter as its text", () => {
    expect(mapKey(keyEvent("a", "KeyA"), mapping())).toEqual({
      action: "press",
      code: "KeyA",
      mods: 0,
      consumedMods: 0,
      text: "a",
      unshiftedCodepoint: code("a"),
    });
  });

  it("marks Shift as used up when it changed what the key typed", () => {
    expect(mapKey(keyEvent("A", "KeyA", { shift: true }), mapping())).toMatchObject({
      text: "A",
      mods: Mod.Shift,
      consumedMods: Mod.Shift,
      unshiftedCodepoint: code("a"),
    });
    expect(mapKey(keyEvent("!", "Digit1", { shift: true }), mapping())).toMatchObject({
      text: "!",
      consumedMods: Mod.Shift,
      unshiftedCodepoint: code("1"),
    });
    // Shift changes nothing about a space, so it is still there to be reported.
    expect(mapKey(keyEvent(" ", "Space", { shift: true }), mapping())).toMatchObject({
      text: " ",
      mods: Mod.Shift,
      consumedMods: 0,
    });
  });

  it("keeps the letter as text under Ctrl and Alt, for the encoder to act on", () => {
    expect(mapKey(keyEvent("c", "KeyC", { ctrl: true }), mapping())).toMatchObject({
      text: "c",
      mods: Mod.Ctrl,
      consumedMods: 0,
      unshiftedCodepoint: code("c"),
    });
    expect(mapKey(keyEvent("a", "KeyA", { alt: true }), mapping())).toMatchObject({
      text: "a",
      mods: Mod.Alt,
    });
  });

  it("gives keys that type nothing no text", () => {
    for (const [key, physical] of [
      ["Enter", "Enter"],
      ["Tab", "Tab"],
      ["Backspace", "Backspace"],
      ["Escape", "Escape"],
      ["ArrowUp", "ArrowUp"],
      ["F5", "F5"],
      ["Delete", "Delete"],
    ] as const) {
      const input = mapKey(keyEvent(key, physical), mapping());
      expect(input, key).toMatchObject({ code: physical, mods: 0 });
      expect(input, key).not.toHaveProperty("text");
      expect(input, key).not.toHaveProperty("unshiftedCodepoint");
    }
  });

  it("treats a character outside the basic plane as one character of text", () => {
    expect(mapKey(keyEvent("😀", ""), mapping())).toMatchObject({ text: "😀" });
  });

  it("reports a held key as a repeat", () => {
    expect(mapKey(keyEvent("a", "KeyA", { repeat: true }), mapping())?.action).toBe("repeat");
  });

  it("reports the locks that are on", () => {
    const caps = mapKey(keyEvent("A", "KeyA", { states: ["CapsLock"] }), mapping());
    expect(caps).toMatchObject({
      text: "A",
      mods: Mod.CapsLock,
      // Caps Lock made the capital, not Shift.
      consumedMods: 0,
      unshiftedCodepoint: code("a"),
    });
    expect(modifiers(keyEvent("a", "KeyA", { states: ["NumLock"] }))).toBe(Mod.NumLock);
    expect(
      modifiers(keyEvent("a", "KeyA", { ctrl: true, shift: true, alt: true, meta: true })),
    ).toBe(Mod.Ctrl | Mod.Shift | Mod.Alt | Mod.Super);
  });
});

describe("keys that are not keys to send", () => {
  it("leaves input-method and dead-key events to the events that follow them", () => {
    expect(mapKey(keyEvent("Dead", "BracketLeft"), mapping())).toBeNull();
    expect(mapKey(keyEvent("Process", "KeyA"), mapping())).toBeNull();
    expect(mapKey(keyEvent("a", "KeyA", { keyCode: 229 }), mapping())).toBeNull();
    expect(mapKey(keyEvent("a", "KeyA", { composing: true }), mapping())).toBeNull();
  });

  it("knows a modifier pressed on its own", () => {
    for (const key of ["Shift", "Control", "Alt", "AltGraph", "Meta", "CapsLock"]) {
      expect(isModifierKey(keyEvent(key, "")), key).toBe(true);
    }
    expect(isModifierKey(keyEvent("a", "KeyA", { ctrl: true }))).toBe(false);
  });
});

describe("AltGr", () => {
  it("types its character without Ctrl and Alt", () => {
    // German: AltGr+Q is "@". The browser reports Ctrl and Alt held.
    const event = keyEvent("@", "KeyQ", { ctrl: true, alt: true, states: ["AltGraph"] });
    expect(mapKey(event, mapping("windows"))).toMatchObject({ text: "@", mods: 0 });
    expect(mapKey(event, mapping("linux"))).toMatchObject({ text: "@", mods: 0 });
  });

  it("is recognised on Windows by its character even when the browser does not name it", () => {
    const event = keyEvent("@", "KeyQ", { ctrl: true, alt: true });
    expect(mapKey(event, mapping("windows"))).toMatchObject({ text: "@", mods: 0 });
    // Elsewhere Ctrl+Alt is two modifiers and nothing more.
    expect(mapKey(event, mapping("linux"))?.mods).toBe(Mod.Ctrl | Mod.Alt);
  });

  it("is not mistaken for Ctrl+Alt with a key's own letter", () => {
    const lower = keyEvent("a", "KeyA", { ctrl: true, alt: true });
    expect(mapKey(lower, mapping("windows"))?.mods).toBe(Mod.Ctrl | Mod.Alt);
    const upper = keyEvent("A", "KeyA", { ctrl: true, alt: true, shift: true });
    expect(mapKey(upper, mapping("windows"))?.mods).toBe(Mod.Ctrl | Mod.Alt | Mod.Shift);
  });
});

describe("Option on a Mac", () => {
  const event = keyEvent("å", "KeyA", { alt: true });

  it("types its own characters unless it is set to act as Alt", () => {
    expect(mapKey(event, mapping("mac"))).toMatchObject({ text: "å", mods: 0 });
  });

  it("sends Alt and the key's own letter when it acts as Alt", () => {
    const asAlt = mapping("mac", { macOptionAsAlt: true });
    expect(mapKey(event, asAlt)).toMatchObject({ text: "a", mods: Mod.Alt });
    const shifted = keyEvent("Å", "KeyA", { alt: true, shift: true });
    expect(mapKey(shifted, asAlt)).toMatchObject({ text: "A", mods: Mod.Alt | Mod.Shift });
  });

  it("is an ordinary Alt everywhere else", () => {
    expect(mapKey(keyEvent("a", "KeyA", { alt: true }), mapping("linux"))?.mods).toBe(Mod.Alt);
  });
});

describe("the keypad", () => {
  it("types digits with Num Lock on", () => {
    const event = keyEvent("1", "Numpad1", { states: ["NumLock"] });
    expect(mapKey(event, mapping())).toMatchObject({
      code: "Numpad1",
      text: "1",
      mods: Mod.NumLock,
    });
  });

  it("moves the cursor with Num Lock off, under the keypad's own key names", () => {
    for (const [key, name] of [
      ["End", "NumpadEnd"],
      ["ArrowDown", "NumpadDown"],
      ["PageDown", "NumpadPageDown"],
      ["ArrowLeft", "NumpadLeft"],
      ["Clear", "NumpadBegin"],
      ["Insert", "NumpadInsert"],
      ["Delete", "NumpadDelete"],
    ] as const) {
      const input = mapKey(keyEvent(key, "Numpad1"), mapping());
      expect(input?.code, key).toBe(name);
      expect(input, key).not.toHaveProperty("text");
    }
  });

  it("keeps the event's code for a name the engine does not have", () => {
    const strict = mapping("windows", { knows: (name) => name !== "NumpadEnd" });
    expect(mapKey(keyEvent("End", "Numpad1"), strict)?.code).toBe("Numpad1");
  });

  it("leaves the ordinary movement keys alone", () => {
    expect(mapKey(keyEvent("End", "End"), mapping())?.code).toBe("End");
  });
});

describe("the keyboard layout", () => {
  it("assumes a US keyboard for a key it knows nothing about", () => {
    const layout = new KeyLayout();
    expect(layout.base("KeyQ")).toBe("q");
    expect(layout.base("Digit7")).toBe("7");
    expect(layout.base("Slash")).toBe("/");
    expect(layout.base("F5")).toBeUndefined();
  });

  it("prefers what the browser says the layout is", () => {
    // French: the key where a US keyboard has Q types "a".
    const french = mapping("windows", { layout: new KeyLayout(new Map([["KeyQ", "a"]])) });
    expect(mapKey(keyEvent("a", "KeyQ", { ctrl: true }), french)).toMatchObject({
      text: "a",
      unshiftedCodepoint: code("a"),
    });
    expect(mapKey(keyEvent("A", "KeyQ", { shift: true }), french)).toMatchObject({
      consumedMods: Mod.Shift,
      unshiftedCodepoint: code("a"),
    });
  });

  it("learns a key's character from seeing it typed with no modifier", () => {
    const learning = mapping();
    // Until then a key outside the US table has no character of its own.
    const before = mapKey(keyEvent("¥", "IntlYen", { ctrl: true }), learning);
    expect(before).toMatchObject({ unshiftedCodepoint: code("¥") });
    expect(learning.layout.base("IntlYen")).toBeUndefined();

    mapKey(keyEvent("¥", "IntlYen", { states: ["NumLock"] }), learning);
    expect(learning.layout.base("IntlYen")).toBe("¥");
    const shifted = mapKey(keyEvent("|", "IntlYen", { shift: true }), learning);
    expect(shifted).toMatchObject({ consumedMods: Mod.Shift, unshiftedCodepoint: code("¥") });
  });

  it("does not learn from a key typed with a modifier or a lock that changes it", () => {
    const learning = mapping();
    mapKey(keyEvent("É", "Semicolon", { shift: true }), learning);
    mapKey(keyEvent("É", "Semicolon", { states: ["CapsLock"] }), learning);
    expect(learning.layout.base("Semicolon")).toBe(";");
  });
});

describe("a key release", () => {
  it("carries the key and its modifiers and no text", () => {
    expect(mapKeyRelease(keyEvent("A", "KeyA", { shift: true }), mapping())).toEqual({
      action: "release",
      code: "KeyA",
      mods: Mod.Shift,
      unshiftedCodepoint: code("a"),
    });
    expect(mapKeyRelease(keyEvent("F5", "F5"), mapping())).toEqual({
      action: "release",
      code: "F5",
      mods: 0,
    });
  });
});

describe("text that did not come from a key", () => {
  it("is a press of no key in particular", () => {
    expect(textInput("日本語")).toEqual({ action: "press", code: "", text: "日本語" });
  });
});
