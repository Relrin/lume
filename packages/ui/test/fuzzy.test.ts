import { describe, expect, it } from "vitest";

import {
  fuzzyMatch,
  QUERY_LETTERS_MAX,
  rank,
  rankAny,
  TEXT_LETTERS_MAX,
  type FuzzyMatch,
} from "../src/palette/fuzzy.ts";

/**
 * The best match as it is defined: every way back is tried for every letter. What this
 * finds is what the matching the page uses finds, which is quicker.
 */
function tryingEveryWay(query: string, text: string): FuzzyMatch | undefined {
  const wanted = [...query.toLowerCase()].filter((letter) => letter.trim() !== "");
  if (wanted.length === 0) return { score: 0, indices: [] };
  const letters = [...text];
  const begins = (at: number) => {
    const [before, here] = [letters[at - 1] ?? "", letters[at] ?? ""];
    if (at === 0 || " -_.:/\\".includes(before)) return true;
    return before !== before.toUpperCase() && here !== here.toLowerCase();
  };
  const best: number[][] = [];
  const from: number[][] = [];
  wanted.forEach((letter, i) => {
    const row = letters.map(() => -Infinity);
    const back = letters.map(() => -1);
    letters.forEach((here, j) => {
      if (here.toLowerCase() !== letter) return;
      const worth = 1 + (begins(j) ? 8 : 0) + (j === 0 ? 4 : 0);
      if (i === 0) row[j] = worth - Math.min(j, 3);
      for (let k = 0; i > 0 && k < j; k++) {
        const score = best[i - 1]![k]! + (k === j - 1 ? 6 : -Math.min(j - k - 1, 3)) + worth;
        if (score > row[j]!) {
          row[j] = score;
          back[j] = k;
        }
      }
    });
    best.push(row);
    from.push(back);
  });
  const last = best.at(-1)!;
  const score = Math.max(...last);
  if (score === -Infinity) return undefined;
  const indices: number[] = [];
  for (let i = wanted.length - 1, j = last.indexOf(score); i >= 0; i--) {
    indices.unshift(j);
    j = from[i]![j]!;
  }
  return { score, indices };
}

/** Numbers that look like chance and are the same each time. */
function chance(seed: number): (below: number) => number {
  let state = seed >>> 0;
  return (below) => {
    state = (Math.imul(state ^ (state >>> 15), 0x2c1b3c6d) + 0x9e3779b9) >>> 0;
    const mixed = Math.imul(state ^ (state >>> 13), 0x297a2d39);
    return ((mixed ^ (mixed >>> 16)) >>> 0) % below;
  };
}

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
    // The letter before the one that begins the word is the space, wherever its halves are.
    expect(fuzzyMatch("b", "😀😀 bar")?.score).toBe(fuzzyMatch("b", "xx bar")?.score);
    expect(fuzzyMatch("b", "😀😀xb-b")?.indices).toEqual([5]);
  });

  it("is the best there is, however it is found", () => {
    const below = chance(7);
    const letters = [..."aabbcABC-_ ./\\é😀"];
    const text = (length: number) =>
      Array.from({ length }, () => letters[below(letters.length)]).join("");
    let matched = 0;
    for (let pair = 0; pair < 20_000; pair++) {
      // A few letters in names like any, and now and then many in a long text.
      const long = pair % 200 === 0;
      const query = text(1 + below(long ? 20 : 5));
      const within = text(below(long ? TEXT_LETTERS_MAX : 24));
      const found = fuzzyMatch(query, within);
      expect(found, `${JSON.stringify(query)} in ${JSON.stringify(within)}`).toEqual(
        tryingEveryWay(query, within),
      );
      if (found && found.indices.length > 1) matched += 1;
    }
    expect(matched).toBeGreaterThan(2000);
  });

  it("looks at the first letters of a long query and of a long text", () => {
    const text = `${"x".repeat(TEXT_LETTERS_MAX - 1)}yz`;
    expect(fuzzyMatch("y", text)?.indices).toEqual([TEXT_LETTERS_MAX - 1]);
    expect(fuzzyMatch("z", text)).toBeUndefined();
    const faces = "😀".repeat(TEXT_LETTERS_MAX);
    expect(fuzzyMatch("z", `${faces}z`)).toBeUndefined();
    expect(fuzzyMatch("z", `${faces.slice(2)}z`)?.indices).toEqual([TEXT_LETTERS_MAX - 1]);
    const asked = "a".repeat(QUERY_LETTERS_MAX);
    expect(fuzzyMatch(`${asked}zzz`, asked)?.indices).toHaveLength(QUERY_LETTERS_MAX);
    // A query that is mostly blanks is its letters.
    expect(fuzzyMatch(`${" ".repeat(1000)}b`, "ab")?.indices).toEqual([1]);
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

  it("counts an item by the text of its that matches best", () => {
    const items = [["none"], ["ab", "a-b", "xaxb"], ["xaxb"], []];
    const ranked = rankAny(items, "ab", (item) => item);
    expect(ranked.map((match) => match.item)).toEqual([items[1], items[2]]);
    expect(ranked[0]?.score).toBe(fuzzyMatch("ab", "a-b")?.score);
    // A match is within one text: the letters of two are not one word.
    expect(rankAny([["a", "b"]], "ab", (item) => item)).toEqual([]);
    expect(rankAny(items, " ", (item) => item)).toHaveLength(items.length);
  });

  it("keeps the order of items that match equally well", () => {
    const ranked = rank(["ab one", "ab two", "ab three"], "ab", (item) => item);
    expect(ranked.map((match) => match.item)).toEqual(["ab one", "ab two", "ab three"]);
  });
});
