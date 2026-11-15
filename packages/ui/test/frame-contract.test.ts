import { describe, expect, it } from "vitest";

import { checkFrameContract, frameContractDifferences } from "../src/frame-contract.ts";

describe("the frame contract", () => {
  // The terminal engine and the renderer each declare what a frame's numbers mean.
  // Nothing but this check stops one from being changed without the other.
  it("has the terminal and the renderer agree on every shared number", () => {
    expect(frameContractDifferences()).toEqual([]);
    expect(() => checkFrameContract()).not.toThrow();
  });
});
