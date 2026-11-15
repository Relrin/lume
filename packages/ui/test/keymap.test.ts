import { describe, expect, it } from "vitest";

import type { Platform } from "../src/terminal/keyboard.ts";
import { systemKey, unboundSystemKey } from "../src/terminal/keymap.ts";
import { keyEvent, type KeyOptions } from "./keys.ts";

function system(
  platform: Platform,
  key: string,
  code: string,
  options: KeyOptions = {},
  developerTools = false,
): boolean {
  return systemKey(keyEvent(key, code, options), { platform, developerTools });
}

describe("the keys left to the system whatever is bound", () => {
  for (const platform of ["windows", "linux"] as const) {
    it(`closing the window and opening its menu stay with the window manager (${platform})`, () => {
      expect(system(platform, "F4", "F4", { alt: true })).toBe(true);
      expect(system(platform, " ", "Space", { alt: true })).toBe(true);
      // With Ctrl as well they are ordinary keys again.
      expect(system(platform, "F4", "F4", { alt: true, ctrl: true })).toBe(false);
      expect(system(platform, "F4", "F4")).toBe(false);
    });
  }

  it("a Mac keeps none back before the bindings have been asked", () => {
    expect(system("mac", "F4", "F4", { alt: true })).toBe(false);
    expect(system("mac", "q", "KeyQ", { meta: true })).toBe(false);
  });

  it("the developer tools key is left alone only in development", () => {
    for (const platform of ["windows", "linux", "mac"] as const) {
      expect(system(platform, "F12", "F12", {}, true)).toBe(true);
      expect(system(platform, "F12", "F12", {}, false)).toBe(false);
    }
  });

  it("ordinary keys are never the system's", () => {
    for (const platform of ["windows", "linux", "mac"] as const) {
      expect(system(platform, "c", "KeyC", { ctrl: true })).toBe(false);
      expect(system(platform, "T", "KeyT", { ctrl: true, shift: true })).toBe(false);
      expect(system(platform, "Enter", "Enter")).toBe(false);
    }
  });
});

describe("a key no binding took", () => {
  it("is the system's when the Command or Windows key is held", () => {
    expect(unboundSystemKey(keyEvent("q", "KeyQ", { meta: true }))).toBe(true);
    expect(unboundSystemKey(keyEvent("Tab", "Tab", { meta: true, shift: true }))).toBe(true);
  });

  it("is the program's otherwise", () => {
    expect(unboundSystemKey(keyEvent("c", "KeyC", { ctrl: true }))).toBe(false);
    expect(unboundSystemKey(keyEvent("b", "KeyB", { alt: true }))).toBe(false);
    expect(unboundSystemKey(keyEvent("a", "KeyA"))).toBe(false);
  });
});
