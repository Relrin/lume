import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { TEXT_MAX_BYTES } from "../src/limits.ts";
import { plain } from "../src/text/plain.ts";

type Range = readonly [string, string];

/** The rule as it is written down for the core and the page alike. */
const RULE = JSON.parse(
  readFileSync(new URL("../../../tests/text/plain.json", import.meta.url), "utf8"),
) as {
  readonly max_bytes: number;
  readonly marks_max: number;
  readonly spaces: readonly Range[];
  readonly dropped: readonly Range[];
  readonly piled: readonly Range[];
  readonly blank: readonly Range[];
  readonly cases: readonly {
    readonly why: string;
    readonly text: string;
    readonly plain: string;
    readonly max?: number;
  }[];
};

const bytes = (text: string) => new TextEncoder().encode(text).length;

/** The characters of some ranges, by number. */
function points(ranges: readonly Range[]): Set<number> {
  const found = new Set<number>();
  for (const [first, last] of ranges) {
    for (let point = parseInt(first, 16); point <= parseInt(last, 16); point += 1) found.add(point);
  }
  return found;
}

describe("a text from outside, as it is shown", () => {
  it("is what the rule's cases say", () => {
    expect(RULE.max_bytes).toBe(TEXT_MAX_BYTES);
    expect(RULE.cases.length).toBeGreaterThan(20);
    for (const { why, text, plain: shown, max } of RULE.cases) {
      expect(plain(text, max), `${why}: ${JSON.stringify(text)}`).toBe(shown);
    }
  });

  it("holds each character there is to the rule", () => {
    const spaces = points(RULE.spaces);
    const dropped = points(RULE.dropped);
    const piled = points(RULE.piled);
    const blank = points(RULE.blank);
    const wrong: string[] = [];
    for (let point = 0; point <= 0x10ffff; point += 1) {
      if (point >= 0xd800 && point <= 0xdfff) continue;
      const character = String.fromCodePoint(point);
      const many = character.repeat(RULE.marks_max + 2);
      const within = spaces.has(point)
        ? " ".repeat(RULE.marks_max + 2)
        : dropped.has(point)
          ? ""
          : piled.has(point)
            ? character.repeat(RULE.marks_max)
            : many;
      const seen = !(spaces.has(point) || dropped.has(point) || piled.has(point));
      const alone = seen && !blank.has(point) ? character : "";
      if (plain(`a${many}b`) !== `a${within}b` || plain(character) !== alone) {
        wrong.push(point.toString(16));
      }
    }
    expect(wrong).toEqual([]);
  });

  it("is cut at the limit, on a character and never inside one", () => {
    expect(plain("x".repeat(TEXT_MAX_BYTES + 500))).toBe("x".repeat(TEXT_MAX_BYTES));
    // Each character is three bytes; the limit falls inside one.
    const title = plain("漢".repeat(TEXT_MAX_BYTES));
    expect(bytes(title)).toBeLessThanOrEqual(TEXT_MAX_BYTES);
    expect(bytes(title)).toBeGreaterThan(TEXT_MAX_BYTES - 3);
    expect([...title].every((character) => character === "漢")).toBe(true);
    // Two UTF-16 units, four bytes: neither half is left behind.
    const faces = plain("😀".repeat(TEXT_MAX_BYTES));
    expect([...faces].every((character) => character === "😀")).toBe(true);
    expect(bytes(faces)).toBe(TEXT_MAX_BYTES);
    expect(plain(`a${"é".repeat(TEXT_MAX_BYTES)}`)).toHaveLength(TEXT_MAX_BYTES / 2);
  });

  it("is the same when it is made safe again", () => {
    for (const { text, max } of RULE.cases) {
      const once = plain(text, max);
      expect(plain(once, max)).toBe(once);
    }
  });
});
