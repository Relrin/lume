import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { KeyEncoder, Mod, type KeyInput, type Terminal, type Vt } from "../src/index.ts";
import { loadVt } from "./support.ts";

let vt: Vt;
let encoder: KeyEncoder;
let term: Terminal;
beforeAll(async () => {
  vt = await loadVt();
  encoder = new KeyEncoder(vt);
});
afterAll(() => encoder.free());

const ESC = "\x1b";
const decoder = new TextDecoder();

/** The bytes a key sends in a fresh terminal after `setup` has been written to it. */
function sent(setup: string, input: Partial<KeyInput> & { code: string }): string {
  term = vt.createTerminal(80, 24);
  try {
    term.write(setup);
    return decoder.decode(encoder.encode(term, { action: "press", ...input }));
  } finally {
    term.free();
  }
}

describe("keys in the default modes", () => {
  it.each([
    ["Enter", "\r"],
    ["Tab", "\t"],
    ["Backspace", "\x7f"],
    ["Escape", ESC],
    ["ArrowUp", `${ESC}[A`],
    ["ArrowDown", `${ESC}[B`],
    ["ArrowRight", `${ESC}[C`],
    ["ArrowLeft", `${ESC}[D`],
    ["Home", `${ESC}[H`],
    ["End", `${ESC}[F`],
    ["Insert", `${ESC}[2~`],
    ["Delete", `${ESC}[3~`],
    ["PageUp", `${ESC}[5~`],
    ["PageDown", `${ESC}[6~`],
    ["F1", `${ESC}OP`],
    ["F4", `${ESC}OS`],
    ["F5", `${ESC}[15~`],
    ["F12", `${ESC}[24~`],
  ])("%s", (code, bytes) => {
    expect(sent("", { code })).toBe(bytes);
  });

  it("sends the text a key produced", () => {
    expect(sent("", { code: "KeyA", text: "a" })).toBe("a");
    expect(sent("", { code: "KeyA", text: "A", mods: Mod.Shift, consumedMods: Mod.Shift })).toBe(
      "A",
    );
    expect(sent("", { code: "Semicolon", text: "ö" })).toBe("ö");
  });

  it("sends text for a key the library has no name for", () => {
    expect(encoder.knows("Lang1")).toBe(false);
    expect(sent("", { code: "Lang1", text: "한" })).toBe("한");
  });

  it("turns Ctrl and a letter into the control character", () => {
    expect(sent("", { code: "KeyC", text: "c", mods: Mod.Ctrl })).toBe("\x03");
    expect(sent("", { code: "KeyD", text: "d", mods: Mod.Ctrl })).toBe("\x04");
    expect(sent("", { code: "Space", text: " ", mods: Mod.Ctrl })).toBe("\x00");
    expect(sent("", { code: "KeyC", text: "c", mods: Mod.Ctrl | Mod.Alt })).toBe(`${ESC}\x03`);
  });

  it("prefixes Alt with escape", () => {
    expect(sent("", { code: "KeyA", text: "a", mods: Mod.Alt })).toBe(`${ESC}a`);
  });

  it("adds modifiers to cursor and function keys", () => {
    expect(sent("", { code: "Tab", mods: Mod.Shift })).toBe(`${ESC}[Z`);
    expect(sent("", { code: "ArrowUp", mods: Mod.Shift })).toBe(`${ESC}[1;2A`);
    expect(sent("", { code: "ArrowRight", mods: Mod.Ctrl })).toBe(`${ESC}[1;5C`);
    expect(sent("", { code: "Delete", mods: Mod.Ctrl | Mod.Shift })).toBe(`${ESC}[3;6~`);
  });

  it("sends nothing for a release, a bare modifier or a composing key", () => {
    expect(sent("", { code: "KeyA", text: "a", action: "release" })).toBe("");
    expect(sent("", { code: "ShiftLeft", mods: Mod.Shift })).toBe("");
    expect(sent("", { code: "KeyA", text: "a", composing: true })).toBe("");
  });

  it("repeats like a press", () => {
    expect(sent("", { code: "ArrowUp", action: "repeat" })).toBe(`${ESC}[A`);
  });
});

describe("keys under modes a program set", () => {
  it("uses application cursor keys", () => {
    expect(sent(`${ESC}[?1h`, { code: "ArrowUp" })).toBe(`${ESC}OA`);
    expect(sent(`${ESC}[?1h${ESC}[?1l`, { code: "ArrowUp" })).toBe(`${ESC}[A`);
  });

  it("sends backspace as control-H when the program asks", () => {
    expect(sent(`${ESC}[?67h`, { code: "Backspace" })).toBe("\x08");
  });

  it("uses the Kitty protocol once the program enables it", () => {
    const kitty = `${ESC}[>1u`;
    expect(sent(kitty, { code: "Escape" })).toBe(`${ESC}[27u`);
    expect(sent(kitty, { code: "KeyC", text: "c", mods: Mod.Ctrl, unshiftedCodepoint: 0x63 })).toBe(
      `${ESC}[99;5u`,
    );
    expect(sent(kitty, { code: "KeyA", text: "a" })).toBe("a");
  });

  it("reports releases when the program asks for event types", () => {
    const events = `${ESC}[>3u`;
    expect(sent(events, { code: "Escape", action: "release" })).toBe(`${ESC}[27;1:3u`);
  });
});

describe("the key table", () => {
  // The codes a browser reports for a full keyboard.
  const codes = [
    ..."ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("").map((c) => `Key${c}`),
    ...Array.from({ length: 10 }, (_, i) => `Digit${i}`),
    ...Array.from({ length: 10 }, (_, i) => `Numpad${i}`),
    ...Array.from({ length: 12 }, (_, i) => `F${i + 1}`),
    ...["Backquote", "Minus", "Equal", "BracketLeft", "BracketRight", "Backslash", "Semicolon"],
    ...["Quote", "Comma", "Period", "Slash", "IntlBackslash", "IntlRo", "IntlYen", "Space"],
    ...["Enter", "Tab", "Backspace", "Escape", "CapsLock", "ContextMenu", "Insert", "Delete"],
    ...["Home", "End", "PageUp", "PageDown", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"],
    ...["ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight", "AltLeft", "AltRight"],
    ...["MetaLeft", "MetaRight", "OSLeft", "OSRight", "NumLock", "ScrollLock", "Pause"],
    ...["PrintScreen", "NumpadAdd", "NumpadSubtract", "NumpadMultiply", "NumpadDivide"],
    ...["NumpadDecimal", "NumpadEnter", "NumpadEqual", "NumpadComma", "Convert", "NonConvert"],
    ...["KanaMode", "AudioVolumeUp", "AudioVolumeDown", "AudioVolumeMute", "MediaPlayPause"],
    ...["MediaTrackNext", "MediaTrackPrevious", "BrowserBack", "BrowserForward", "LaunchApp1"],
  ];

  it.each(codes)("knows %s", (code) => {
    expect(encoder.knows(code)).toBe(true);
  });

  it("refuses use after free", () => {
    const other = new KeyEncoder(vt);
    other.free();
    other.free();
    const t = vt.createTerminal(4, 2);
    expect(() => other.encode(t, { action: "press", code: "KeyA" })).toThrow(/freed/);
    t.free();
  });
});
