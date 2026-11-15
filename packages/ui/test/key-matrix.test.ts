import { readFileSync } from "node:fs";

import { KeyEncoder, Vt, type Terminal } from "@lume/vt";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import {
  KeyLayout,
  mapKey,
  mapKeyRelease,
  textInput,
  type KeyEventLike,
  type KeyMapping,
} from "../src/terminal/keyboard.ts";
import { keyEvent, type KeyOptions } from "./keys.ts";

/**
 * What a keyboard event sends, from the event a browser raises to the bytes a program
 * reads: the shell's reading of the event followed by the engine's encoding of the key.
 * Each table is one keyboard mode a program can ask for.
 */

const ESC = "\x1b";

let vt: Vt;
let keys: KeyEncoder;
const terminals: Terminal[] = [];

beforeAll(async () => {
  let bytes: Uint8Array;
  try {
    bytes = readFileSync(new URL("../../vt/dist/ghostty-vt.wasm", import.meta.url));
  } catch {
    throw new Error("the terminal engine is not built. Run `cargo xtask vt wasm` first.");
  }
  vt = await Vt.instantiate(bytes);
  keys = new KeyEncoder(vt);
});

afterAll(() => {
  for (const terminal of terminals) terminal.free();
  keys.free();
});

/** A terminal after a program has written `setup` to it. */
function terminalAfter(setup: string): Terminal {
  const terminal = vt.createTerminal(80, 24);
  terminals.push(terminal);
  terminal.write(setup);
  return terminal;
}

function mapping(): KeyMapping {
  return {
    platform: "windows",
    macOptionAsAlt: false,
    layout: new KeyLayout(),
    knows: (code) => keys.knows(code),
  };
}

const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

function press(terminal: Terminal, event: KeyEventLike): string {
  const input = mapKey(event, mapping());
  if (!input) throw new Error(`${event.key} is not a key to send`);
  return text(keys.encode(terminal, input));
}

function release(terminal: Terminal, event: KeyEventLike): string {
  return text(keys.encode(terminal, mapKeyRelease(event, mapping())));
}

type Row = readonly [name: string, key: string, code: string, options: KeyOptions, sends: string];

const CTRL = { ctrl: true };
const SHIFT = { shift: true };
const ALT = { alt: true };

function check(terminal: () => Terminal, rows: readonly Row[]): void {
  for (const [name, key, code, options, sends] of rows) {
    it(`${name} sends ${JSON.stringify(sends)}`, () => {
      expect(press(terminal(), keyEvent(key, code, options))).toBe(sends);
    });
  }
}

describe("keys in a terminal no program has configured", () => {
  let terminal: Terminal;
  beforeAll(() => (terminal = terminalAfter("")));

  check(
    () => terminal,
    [
      ["a", "a", "KeyA", {}, "a"],
      ["Shift+A", "A", "KeyA", SHIFT, "A"],
      ["a with Caps Lock", "A", "KeyA", { states: ["CapsLock"] }, "A"],
      ["Space", " ", "Space", {}, " "],
      ["Shift+1", "!", "Digit1", SHIFT, "!"],
      ["é", "é", "Digit2", {}, "é"],
      ["Enter", "Enter", "Enter", {}, "\r"],
      ["Tab", "Tab", "Tab", {}, "\t"],
      ["Shift+Tab", "Tab", "Tab", SHIFT, `${ESC}[Z`],
      ["Backspace", "Backspace", "Backspace", {}, "\x7f"],
      ["Escape", "Escape", "Escape", {}, ESC],
      ["Ctrl+C", "c", "KeyC", CTRL, "\x03"],
      ["Ctrl+D", "d", "KeyD", CTRL, "\x04"],
      ["Ctrl+Z", "z", "KeyZ", CTRL, "\x1a"],
      ["Ctrl+Space", " ", "Space", CTRL, "\x00"],
      ["Ctrl+Backspace", "Backspace", "Backspace", CTRL, "\x08"],
      ["Alt+A", "a", "KeyA", ALT, `${ESC}a`],
      ["Alt+Backspace", "Backspace", "Backspace", ALT, `${ESC}\x7f`],
      ["AltGr+Q on a German keyboard", "@", "KeyQ", { ctrl: true, alt: true }, "@"],
      ["Up", "ArrowUp", "ArrowUp", {}, `${ESC}[A`],
      ["Down", "ArrowDown", "ArrowDown", {}, `${ESC}[B`],
      ["Right", "ArrowRight", "ArrowRight", {}, `${ESC}[C`],
      ["Left", "ArrowLeft", "ArrowLeft", {}, `${ESC}[D`],
      ["Shift+Right", "ArrowRight", "ArrowRight", SHIFT, `${ESC}[1;2C`],
      ["Alt+Left", "ArrowLeft", "ArrowLeft", ALT, `${ESC}[1;3D`],
      ["Ctrl+Up", "ArrowUp", "ArrowUp", CTRL, `${ESC}[1;5A`],
      ["Ctrl+Shift+Left", "ArrowLeft", "ArrowLeft", { ctrl: true, shift: true }, `${ESC}[1;6D`],
      ["Home", "Home", "Home", {}, `${ESC}[H`],
      ["End", "End", "End", {}, `${ESC}[F`],
      ["Insert", "Insert", "Insert", {}, `${ESC}[2~`],
      ["Delete", "Delete", "Delete", {}, `${ESC}[3~`],
      ["Page Up", "PageUp", "PageUp", {}, `${ESC}[5~`],
      ["Page Down", "PageDown", "PageDown", {}, `${ESC}[6~`],
      ["Ctrl+Delete", "Delete", "Delete", CTRL, `${ESC}[3;5~`],
      ["F1", "F1", "F1", {}, `${ESC}OP`],
      ["F4", "F4", "F4", {}, `${ESC}OS`],
      ["F5", "F5", "F5", {}, `${ESC}[15~`],
      ["F12", "F12", "F12", {}, `${ESC}[24~`],
      ["Shift+F5", "F5", "F5", SHIFT, `${ESC}[15;2~`],
      ["keypad 1 with Num Lock", "1", "Numpad1", { states: ["NumLock"] }, "1"],
      ["keypad End without Num Lock", "End", "Numpad1", {}, `${ESC}[F`],
      ["keypad Enter", "Enter", "NumpadEnter", {}, "\r"],
    ],
  );

  it("sends nothing for a modifier on its own or for a key let go", () => {
    expect(press(terminal, keyEvent("Shift", "ShiftLeft", SHIFT))).toBe("");
    expect(press(terminal, keyEvent("Control", "ControlLeft", CTRL))).toBe("");
    expect(release(terminal, keyEvent("a", "KeyA"))).toBe("");
    expect(release(terminal, keyEvent("Enter", "Enter"))).toBe("");
  });

  it("sends text that came from an input method as it is", () => {
    expect(text(keys.encode(terminal, textInput("日本語")))).toBe("日本語");
    expect(text(keys.encode(terminal, textInput("é")))).toBe("é");
  });

  it("sends a held key again", () => {
    expect(press(terminal, keyEvent("a", "KeyA", { repeat: true }))).toBe("a");
    expect(press(terminal, keyEvent("ArrowUp", "ArrowUp", { repeat: true }))).toBe(`${ESC}[A`);
  });
});

describe("keys after a program has asked for application cursor keys", () => {
  let terminal: Terminal;
  beforeAll(() => (terminal = terminalAfter(`${ESC}[?1h`)));

  check(
    () => terminal,
    [
      ["Up", "ArrowUp", "ArrowUp", {}, `${ESC}OA`],
      ["Left", "ArrowLeft", "ArrowLeft", {}, `${ESC}OD`],
      ["Home", "Home", "Home", {}, `${ESC}OH`],
      ["End", "End", "End", {}, `${ESC}OF`],
      // With a modifier the longer form is used, as it has nowhere else to say so.
      ["Ctrl+Up", "ArrowUp", "ArrowUp", CTRL, `${ESC}[1;5A`],
      ["a", "a", "KeyA", {}, "a"],
      ["Ctrl+C", "c", "KeyC", CTRL, "\x03"],
    ],
  );
});

describe("keys after a program has asked for unambiguous key reports", () => {
  let terminal: Terminal;
  beforeAll(() => (terminal = terminalAfter(`${ESC}[>1u`)));

  check(
    () => terminal,
    [
      // Text is still text.
      ["a", "a", "KeyA", {}, "a"],
      ["Shift+A", "A", "KeyA", SHIFT, "A"],
      ["AltGr+Q on a German keyboard", "@", "KeyQ", { ctrl: true, alt: true }, "@"],
      ["Enter", "Enter", "Enter", {}, "\r"],
      ["Tab", "Tab", "Tab", {}, "\t"],
      ["Backspace", "Backspace", "Backspace", {}, "\x7f"],
      // Keys that would otherwise look like something else are spelt out, and named
      // by the character the key types without modifiers, whatever the layout.
      ["Escape", "Escape", "Escape", {}, `${ESC}[27u`],
      ["Ctrl+C", "c", "KeyC", CTRL, `${ESC}[99;5u`],
      ["Ctrl+Shift+C", "C", "KeyC", { ctrl: true, shift: true }, `${ESC}[99;6u`],
      ["Alt+A", "a", "KeyA", ALT, `${ESC}[97;3u`],
      ["Ctrl+Enter", "Enter", "Enter", CTRL, `${ESC}[13;5u`],
      ["Shift+Tab", "Tab", "Tab", SHIFT, `${ESC}[9;2u`],
      ["Up", "ArrowUp", "ArrowUp", {}, `${ESC}[A`],
      ["Ctrl+Up", "ArrowUp", "ArrowUp", CTRL, `${ESC}[1;5A`],
      ["F5", "F5", "F5", {}, `${ESC}[15~`],
    ],
  );

  it("names Ctrl+letter by the layout's letter, not the key's place", () => {
    // French: the key where a US keyboard has Q types "a".
    const french: KeyMapping = { ...mapping(), layout: new KeyLayout(new Map([["KeyQ", "a"]])) };
    const input = mapKey(keyEvent("a", "KeyQ", CTRL), french)!;
    expect(text(keys.encode(terminal, input))).toBe(`${ESC}[97;5u`);
  });

  it("still sends nothing for a key let go", () => {
    expect(release(terminal, keyEvent("a", "KeyA"))).toBe("");
    expect(release(terminal, keyEvent("Escape", "Escape"))).toBe("");
  });
});

describe("keys after a program has also asked to hear repeats and releases", () => {
  let terminal: Terminal;
  beforeAll(() => (terminal = terminalAfter(`${ESC}[>3u`)));

  it("reports a function key held and let go", () => {
    expect(press(terminal, keyEvent("F5", "F5"))).toBe(`${ESC}[15~`);
    expect(press(terminal, keyEvent("F5", "F5", { repeat: true }))).toBe(`${ESC}[15;1:2~`);
    expect(release(terminal, keyEvent("F5", "F5"))).toBe(`${ESC}[15;1:3~`);
  });

  it("reports an arrow key let go with its modifiers", () => {
    expect(release(terminal, keyEvent("ArrowUp", "ArrowUp", CTRL))).toBe(`${ESC}[1;5:3A`);
  });

  it("reports Escape let go", () => {
    expect(press(terminal, keyEvent("Escape", "Escape"))).toBe(`${ESC}[27u`);
    expect(release(terminal, keyEvent("Escape", "Escape"))).toBe(`${ESC}[27;1:3u`);
  });

  it("does not report the release of Enter, Tab or Backspace, which were sent as text", () => {
    for (const key of ["Enter", "Tab", "Backspace"]) {
      expect(release(terminal, keyEvent(key, key)), key).toBe("");
    }
  });
});

describe("keys after a program has asked for every key as a report", () => {
  let terminal: Terminal;
  // Unambiguous reports, event types and all keys as reports.
  beforeAll(() => (terminal = terminalAfter(`${ESC}[>11u`)));

  it("reports letters by number, pressed and let go", () => {
    expect(press(terminal, keyEvent("a", "KeyA"))).toBe(`${ESC}[97u`);
    expect(release(terminal, keyEvent("a", "KeyA"))).toBe(`${ESC}[97;1:3u`);
    expect(press(terminal, keyEvent("A", "KeyA", SHIFT))).toBe(`${ESC}[97;2u`);
  });

  it("reports Enter, Tab and Backspace, pressed and let go", () => {
    expect(press(terminal, keyEvent("Enter", "Enter"))).toBe(`${ESC}[13u`);
    expect(release(terminal, keyEvent("Enter", "Enter"))).toBe(`${ESC}[13;1:3u`);
    expect(press(terminal, keyEvent("Tab", "Tab"))).toBe(`${ESC}[9u`);
    expect(press(terminal, keyEvent("Backspace", "Backspace"))).toBe(`${ESC}[127u`);
  });

  it("reports a modifier on its own", () => {
    expect(press(terminal, keyEvent("Shift", "ShiftLeft", SHIFT))).toBe(`${ESC}[57441;2u`);
    expect(release(terminal, keyEvent("Shift", "ShiftLeft"))).toBe(`${ESC}[57441;1:3u`);
  });
});
