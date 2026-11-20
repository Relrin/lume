import { describe, expect, it } from "vitest";

import { PINS_MAX, readPins, togglePin } from "../src/connect/pins.ts";

/** A page's storage, held in a map. */
function storage(): Pick<Storage, "getItem" | "setItem"> {
  const held = new Map<string, string>();
  return {
    getItem: (key) => held.get(key) ?? null,
    setItem: (key, value) => void held.set(key, value),
  };
}

describe("pinned profiles", () => {
  it("pins and unpins, keeping the order they were pinned in", () => {
    const kept = storage();
    togglePin("box", kept);
    togglePin("nu", kept);
    expect(readPins(kept)).toEqual(["box", "nu"]);
    expect(togglePin("box", kept)).toEqual(["nu"]);
    expect(readPins(kept)).toEqual(["nu"]);
  });

  it("keeps at most a few, dropping the oldest", () => {
    const kept = storage();
    for (let at = 0; at <= PINS_MAX; at += 1) togglePin(`p${at}`, kept);
    const pins = readPins(kept);
    expect(pins).toHaveLength(PINS_MAX);
    expect(pins[0]).toBe("p1");
  });

  it("has no pins where the storage holds something else or cannot be read", () => {
    const odd = storage();
    odd.setItem("lume.connect.pins", "{not json");
    expect(readPins(odd)).toEqual([]);
    expect(readPins(undefined)).toEqual([]);
  });
});
