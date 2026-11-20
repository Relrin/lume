import { describe, expect, it } from "vitest";

import { keyCaps } from "../src/palette/KeyCaps.tsx";

describe("key caps", () => {
  it("cuts a chord into its keys", () => {
    expect(keyCaps("Ctrl+Shift+P")).toEqual([["Ctrl", "Shift", "P"]]);
  });

  it("keeps the key that types a plus whole", () => {
    expect(keyCaps("Ctrl++")).toEqual([["Ctrl", "+"]]);
  });

  it("cuts a sequence into its chords", () => {
    expect(keyCaps("Ctrl+K Ctrl+T")).toEqual([
      ["Ctrl", "K"],
      ["Ctrl", "T"],
    ]);
  });

  it("reads a Mac's signs one by one", () => {
    expect(keyCaps("⌘⇧P")).toEqual([["⌘", "⇧", "P"]]);
    expect(keyCaps("F11")).toEqual([["F11"]]);
  });
});
