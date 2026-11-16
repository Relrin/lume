import { describe, expect, it } from "vitest";

import { fuzzyMatch, rank } from "../src/palette/fuzzy.ts";

describe("matching a query against a text", () => {
  it("matches letters that are in the text in order, whatever their case", () => {
    expect(fuzzyMatch("nt", "New Tab")?.indices).toEqual([0, 4]);
    expect(fuzzyMatch("NEWTAB", "new tab")?.indices).toEqual([0, 1, 2, 4, 5, 6]);
  });

  it("does not match letters that are missing or out of order", () => {
    expect(fuzzyMatch("tn", "New Tab")).toBeUndefined();
    expect(fuzzyMatch("newtabs", "New Tab")).toBeUndefined();
    expect(fuzzyMatch("x", "")).toBeUndefined();
  });

  it("matches everything with an empty query and marks nothing", () => {
    expect(fuzzyMatch("", "Split Right")).toEqual({ score: 0, indices: [] });
    expect(fuzzyMatch("   ", "Split Right")).toEqual({ score: 0, indices: [] });
  });

  it("passes over spaces in the query", () => {
    expect(fuzzyMatch("new tab", "New Tab")?.indices).toEqual([0, 1, 2, 4, 5, 6]);
  });

  it("prefers the letters that begin words to letters inside them", () => {
    // The `s` and `r` of the two words, not the `r` inside "Split Right"'s first word.
    expect(fuzzyMatch("sr", "Split Right")?.indices).toEqual([0, 6]);
    // The capital inside a word begins a word too.
    expect(fuzzyMatch("pv", "openPaneView")?.indices).toEqual([4, 8]);
  });

  it("prefers letters that follow one another", () => {
    const together = fuzzyMatch("tab", "Close Tab")!;
    const apart = fuzzyMatch("tab", "Toggle Pane Bar")!;
    expect(together.indices).toEqual([6, 7, 8]);
    expect(together.score).toBeGreaterThan(apart.score);
  });

  it("counts characters, not their halves", () => {
    expect(fuzzyMatch("b", "😀 bar")?.indices).toEqual([2]);
  });
});

describe("ranking items by a query", () => {
  const commands = ["Tab: New Tab", "Tab: Close Tab", "Pane: Split Right", "Window: New Window"];

  it("leaves out what does not match and puts the best match first", () => {
    const ranked = rank(commands, "nw", (command) => command);
    expect(ranked.map((match) => match.item)).toEqual(["Window: New Window", "Tab: New Tab"]);
  });

  it("keeps the order it was given for an empty query", () => {
    expect(rank(commands, "", (command) => command).map((match) => match.item)).toEqual(commands);
  });

  it("keeps the order of items that match equally well", () => {
    const ranked = rank(["ab one", "ab two", "ab three"], "ab", (item) => item);
    expect(ranked.map((match) => match.item)).toEqual(["ab one", "ab two", "ab three"]);
  });
});
