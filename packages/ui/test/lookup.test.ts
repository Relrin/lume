import { describe, expect, it } from "vitest";

import { commandGroup } from "../src/commands/groups.ts";
import { testFailure } from "../src/connections/model.ts";
import { lookup } from "../src/lookup.ts";
import { buildOf, sessionName, standing, systemName } from "../src/rex/model.ts";
import { rexFailed } from "./rex-fixtures.ts";

/** What every object answers to, whatever is in it. */
const INHERITED = ["constructor", "__proto__", "toString", "hasOwnProperty", "valueOf"];

/** Those of them that the first part of a platform's name may be. */
const SYSTEMS = INHERITED.filter((word) => !word.includes("_"));

describe("a word from outside", () => {
  it("is looked up among a table's own entries", () => {
    expect(lookup({ a: 1 }, "a")).toBe(1);
    expect(lookup({ a: 1 }, "b")).toBeUndefined();
    for (const word of INHERITED) expect(lookup({ a: 1 }, word), word).toBeUndefined();
  });

  it("is never something that cannot be shown", () => {
    for (const word of SYSTEMS) expect(systemName(`${word}_amd64`), word).toBe(word);
    for (const word of INHERITED) {
      expect(standing({ state: rexFailed(word, "x"), sessions: [] }), word).toBe("Failed");
      expect(testFailure(word, "x").short, word).toBe("It did not work");
      expect(commandGroup(word), word).toBe("App");
      expect(commandGroup(`lume.${word}.x`), word).toBe("App");
    }
  });

  it("is shown without what is not seen", () => {
    expect(systemName("\u202eplan9_mips")).toBe("plan9");
    expect(sessionName({ label: "prod\u202egnp.exe" })).toBe("prodgnp.exe");
    expect(sessionName({ label: "\u3164\u200b" })).toBe("unnamed");
    expect(
      buildOf({
        kind: "connected",
        version: "1\n2",
        platform: "\u2066linux",
        api_revision: 1,
      }),
    ).toBe("Rex 1 2 on linux");
  });
});
