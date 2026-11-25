import { describe, expect, it } from "vitest";

import { failureMessage } from "../src/ErrorBoundary.tsx";

const UNKNOWN = "Nothing more is known.";

describe("what is said of something that could not be drawn", () => {
  it("is what was thrown, in words", () => {
    expect(failureMessage(new TypeError("x is not a function"))).toBe("x is not a function");
    expect(failureMessage("a word")).toBe("a word");
    expect(failureMessage(undefined)).toBe("undefined");
  });

  it("is only what may be shown, and only so much of it", () => {
    expect(failureMessage(new Error("one\ntwo\u202e"))).toBe("one two");
    expect(failureMessage(new Error("x".repeat(5000)))).toBe("x".repeat(512));
  });

  it("is something, whatever was thrown", () => {
    expect(failureMessage(new Error(""))).toBe(UNKNOWN);
    expect(failureMessage(Object.create(null))).toBe(UNKNOWN);
    const said = new Error("x");
    Object.defineProperty(said, "message", { get: () => failureMessage(null) + said.message });
    expect(failureMessage(said)).toBe(UNKNOWN);
    Object.defineProperty(said, "message", { value: 7 });
    expect(typeof failureMessage(said)).toBe("string");
  });
});
