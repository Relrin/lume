import { describe, expect, it } from "vitest";

import { CreditError, Granter } from "../src/transport/credit.ts";

describe("the credit granter", () => {
  it("opens a stream with one window", () => {
    const granter = new Granter(1000);
    expect(granter.firstGrant).toBe(1000);
    expect(granter.outstanding).toBe(1000);
  });

  it("grants again only once half a window has been consumed", () => {
    const granter = new Granter(1000);
    granter.received(600);
    expect(granter.consumed(499)).toBe(0);
    expect(granter.consumed(1)).toBe(500);
    expect(granter.outstanding).toBe(900);
    expect(granter.consumed(100)).toBe(0);
  });

  it("rounds half of an odd window up", () => {
    const granter = new Granter(5);
    granter.received(5);
    expect(granter.consumed(2)).toBe(0);
    expect(granter.consumed(1)).toBe(3);
  });

  it("never has more than one window outstanding", () => {
    const granter = new Granter(64);
    for (let round = 0; round < 1000; round++) {
      const take = Math.min(granter.outstanding, 1 + (round % 40));
      granter.received(take);
      granter.consumed(take);
      expect(granter.outstanding).toBeLessThanOrEqual(64);
    }
  });

  it("refuses output beyond what was granted", () => {
    const granter = new Granter(10);
    granter.received(10);
    expect(() => granter.received(1)).toThrow(CreditError);
  });

  it("refuses a window that is not a positive whole number", () => {
    expect(() => new Granter(0)).toThrow(RangeError);
    expect(() => new Granter(1.5)).toThrow(RangeError);
  });
});
