import { describe, expect, it } from "vitest";

import { listed, moreLine } from "../src/listed.ts";

const NUMBERS = Array.from({ length: 10 }, (_, index) => index);

describe("what is listed of a long list", () => {
  it("is all of one that is not too long", () => {
    expect(listed(NUMBERS, 10)).toEqual({ rows: NUMBERS, more: 0 });
    expect(listed([], 3, () => true)).toEqual({ rows: [], more: 0 });
  });

  it("is its first, and how many there are besides", () => {
    expect(listed(NUMBERS, 3)).toEqual({ rows: [0, 1, 2], more: 7 });
    expect(moreLine(7)).toBe("and 7 more");
  });

  it("is what is kept too, wherever it stands", () => {
    expect(listed(NUMBERS, 3, (one) => one === 8)).toEqual({ rows: [0, 1, 8], more: 7 });
    expect(listed(NUMBERS, 3, (one) => one === 1)).toEqual({ rows: [0, 1, 2], more: 7 });
    expect(listed(NUMBERS, 3, (one) => one === 42)).toEqual({ rows: [0, 1, 2], more: 7 });
  });
});
