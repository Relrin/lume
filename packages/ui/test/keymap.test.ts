import { describe, expect, it } from "vitest";

import type { Platform } from "../src/terminal/keyboard.ts";
import { keyPurpose, type KeyPurpose } from "../src/terminal/keymap.ts";
import { keyEvent, type KeyOptions } from "./keys.ts";

function purpose(
  platform: Platform,
  key: string,
  code: string,
  options: KeyOptions = {},
  context: { alternateScreen?: boolean; developerTools?: boolean } = {},
): KeyPurpose {
  return keyPurpose(keyEvent(key, code, options), {
    platform,
    alternateScreen: context.alternateScreen ?? false,
    developerTools: context.developerTools ?? false,
  });
}

const CTRL_SHIFT = { ctrl: true, shift: true };

describe("the keys the shell keeps on Windows and Linux", () => {
  for (const platform of ["windows", "linux"] as const) {
    it(`copies and pastes with Ctrl+Shift and with Insert (${platform})`, () => {
      expect(purpose(platform, "C", "KeyC", CTRL_SHIFT)).toBe("copy");
      expect(purpose(platform, "V", "KeyV", CTRL_SHIFT)).toBe("paste");
      expect(purpose(platform, "Insert", "Insert", { ctrl: true })).toBe("copy");
      expect(purpose(platform, "Insert", "Insert", { shift: true })).toBe("paste");
    });

    it(`leaves Ctrl+C and Ctrl+V to the program (${platform})`, () => {
      expect(purpose(platform, "c", "KeyC", { ctrl: true })).toBeNull();
      expect(purpose(platform, "v", "KeyV", { ctrl: true })).toBeNull();
      expect(purpose(platform, "Insert", "Insert")).toBeNull();
    });

    it(`changes the font size with Ctrl and plus, minus and zero (${platform})`, () => {
      expect(purpose(platform, "=", "Equal", { ctrl: true })).toBe("font-larger");
      expect(purpose(platform, "+", "Equal", CTRL_SHIFT)).toBe("font-larger");
      expect(purpose(platform, "+", "NumpadAdd", { ctrl: true })).toBe("font-larger");
      expect(purpose(platform, "-", "Minus", { ctrl: true })).toBe("font-smaller");
      expect(purpose(platform, "0", "Digit0", { ctrl: true })).toBe("font-reset");
      // Ctrl+Shift+minus is Ctrl+underscore, which programs use.
      expect(purpose(platform, "_", "Minus", CTRL_SHIFT)).toBeNull();
    });

    it(`scrolls back with Shift and the paging keys (${platform})`, () => {
      expect(purpose(platform, "PageUp", "PageUp", { shift: true })).toBe("scroll-page-up");
      expect(purpose(platform, "PageDown", "PageDown", { shift: true })).toBe("scroll-page-down");
      expect(purpose(platform, "Home", "Home", { shift: true })).toBe("scroll-top");
      expect(purpose(platform, "End", "End", { shift: true })).toBe("scroll-bottom");
      expect(purpose(platform, "PageUp", "PageUp")).toBeNull();
    });

    it(`clears with Ctrl+Shift+K (${platform})`, () => {
      expect(purpose(platform, "K", "KeyK", CTRL_SHIFT)).toBe("clear");
      expect(purpose(platform, "k", "KeyK", { ctrl: true })).toBeNull();
    });

    it(`leaves the window manager's keys alone (${platform})`, () => {
      expect(purpose(platform, "F4", "F4", { alt: true })).toBe("system");
      expect(purpose(platform, " ", "Space", { alt: true })).toBe("system");
      expect(purpose(platform, "e", "KeyE", { meta: true })).toBe("system");
    });

    it(`sends everything else to the program (${platform})`, () => {
      expect(purpose(platform, "a", "KeyA")).toBeNull();
      expect(purpose(platform, "F5", "F5")).toBeNull();
      expect(purpose(platform, "r", "KeyR", { ctrl: true })).toBeNull();
      expect(purpose(platform, "a", "KeyA", { ctrl: true })).toBeNull();
      expect(purpose(platform, "F12", "F12")).toBeNull();
    });
  }
});

describe("the keys the shell keeps on a Mac", () => {
  const cmd = { meta: true };

  it("copies, pastes, clears and selects everything with Command", () => {
    expect(purpose("mac", "c", "KeyC", cmd)).toBe("copy");
    expect(purpose("mac", "v", "KeyV", cmd)).toBe("paste");
    expect(purpose("mac", "k", "KeyK", cmd)).toBe("clear");
    expect(purpose("mac", "a", "KeyA", cmd)).toBe("select-all");
  });

  it("changes the font size with Command and plus, minus and zero", () => {
    expect(purpose("mac", "=", "Equal", cmd)).toBe("font-larger");
    expect(purpose("mac", "+", "Equal", { meta: true, shift: true })).toBe("font-larger");
    expect(purpose("mac", "-", "Minus", cmd)).toBe("font-smaller");
    expect(purpose("mac", "0", "Digit0", cmd)).toBe("font-reset");
  });

  it("scrolls back with Shift or Command and the paging keys", () => {
    expect(purpose("mac", "PageUp", "PageUp", { shift: true })).toBe("scroll-page-up");
    expect(purpose("mac", "PageUp", "PageUp", cmd)).toBe("scroll-page-up");
    expect(purpose("mac", "Home", "Home", cmd)).toBe("scroll-top");
    expect(purpose("mac", "End", "End", cmd)).toBe("scroll-bottom");
  });

  it("leaves every other Command key to the system and never to the program", () => {
    expect(purpose("mac", "q", "KeyQ", cmd)).toBe("system");
    expect(purpose("mac", "w", "KeyW", cmd)).toBe("system");
    expect(purpose("mac", "Tab", "Tab", cmd)).toBe("system");
    expect(purpose("mac", "c", "KeyC", { meta: true, alt: true })).toBe("system");
  });

  it("sends Ctrl+C to the program, where Ctrl+Shift+C is nothing special", () => {
    expect(purpose("mac", "c", "KeyC", { ctrl: true })).toBeNull();
    expect(purpose("mac", "C", "KeyC", CTRL_SHIFT)).toBeNull();
  });
});

describe("shortcuts and the keyboard layout", () => {
  it("follow the letter a key types", () => {
    // Dvorak: the key where a US keyboard has I types "c".
    expect(purpose("windows", "C", "KeyI", CTRL_SHIFT)).toBe("copy");
    expect(purpose("windows", "X", "KeyC", CTRL_SHIFT)).toBeNull();
  });

  it("fall back to the key's place on a layout that types no such letter", () => {
    // Russian: the C key types a Cyrillic letter.
    expect(purpose("windows", "С", "KeyC", CTRL_SHIFT)).toBe("copy");
    expect(purpose("mac", "с", "KeyC", { meta: true })).toBe("copy");
  });
});

describe("scrolling keys in a full-screen program", () => {
  it("belong to the program, which has no scrollback of the terminal's to show", () => {
    const alternate = { alternateScreen: true };
    expect(purpose("windows", "PageUp", "PageUp", { shift: true }, alternate)).toBeNull();
    expect(purpose("windows", "Home", "Home", { shift: true }, alternate)).toBeNull();
    expect(purpose("mac", "PageUp", "PageUp", { shift: true }, alternate)).toBeNull();
    // Copy and paste still work there.
    expect(purpose("windows", "C", "KeyC", CTRL_SHIFT, alternate)).toBe("copy");
  });
});

describe("the developer tools key", () => {
  it("is left to the webview only where the tools exist", () => {
    expect(purpose("windows", "F12", "F12", {}, { developerTools: true })).toBe("system");
    expect(purpose("windows", "F12", "F12", {}, { developerTools: false })).toBeNull();
  });
});
