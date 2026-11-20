import { describe, expect, it } from "vitest";

import { readSign, stepScope } from "../src/palette/scopes.ts";

describe("what the command palette searches", () => {
  it("reads the sign typed first", () => {
    expect(readSign(">split")).toEqual({ scope: "commands", rest: "split" });
    expect(readSign("@ claude")).toEqual({ scope: "tabs", rest: "claude" });
    expect(readSign("#box")).toEqual({ scope: "hosts", rest: "box" });
    expect(readSign("split")).toBeNull();
    expect(readSign("/src")).toBeNull();
  });

  it("steps round the scopes both ways", () => {
    expect(stepScope("all", 1)).toBe("commands");
    expect(stepScope("hosts", 1)).toBe("all");
    expect(stepScope("all", -1)).toBe("hosts");
  });
});
