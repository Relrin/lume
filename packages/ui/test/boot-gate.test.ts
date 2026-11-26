import { describe, expect, it } from "vitest";

import type { Notice } from "../src/core/types.ts";
import { CoreGate } from "../src/shell/boot/gate.ts";

/** A notice told apart from others by the message it carries. */
function said(message: string): Notice {
  return { type: "notice", level: "info", message };
}

/** What the gate passed on, in the order it did. */
function record() {
  const seen: string[] = [];
  return {
    seen,
    follow: (notice: Notice) => {
      if (notice.type === "notice") seen.push(`notice ${notice.message}`);
    },
    answer: (message: unknown) => void seen.push(`request ${String(message)}`),
  };
}

describe("the gate between the core and a starting page", () => {
  it("keeps what the core says before the page is live", () => {
    const gate = new CoreGate();
    const page = record();
    gate.handlers.notice(said("one"));
    gate.handlers.notice(said("two"));
    expect(page.seen).toEqual([]);
    expect(gate.live).toBe(false);
    gate.goLive(page.follow, page.answer);
    expect(gate.live).toBe(true);
    expect(page.seen).toEqual(["notice one", "notice two"]);
  });

  it("follows what is said once live at once, after what was kept", () => {
    const gate = new CoreGate();
    const page = record();
    gate.notice(said("early"));
    gate.goLive(page.follow, page.answer);
    gate.notice(said("late"));
    expect(page.seen).toEqual(["notice early", "notice late"]);
  });

  it("holds what the core asks until the page is live, and answers it after the notices", () => {
    const gate = new CoreGate();
    const page = record();
    gate.handlers.request("first");
    gate.handlers.notice(said("between"));
    gate.handlers.request("second");
    expect(page.seen).toEqual([]);
    gate.goLive(page.follow, page.answer);
    // The page follows what it was told before it answers anything: an answer may
    // depend on a change the core announced first.
    expect(page.seen).toEqual(["notice between", "request first", "request second"]);
    gate.request("third");
    expect(page.seen.at(-1)).toBe("request third");
  });

  it("lets what is asked go when the page answers nothing", () => {
    const gate = new CoreGate();
    const page = record();
    gate.request("unanswered");
    gate.notice(said("kept"));
    gate.goLive(page.follow);
    gate.request("also unanswered");
    expect(page.seen).toEqual(["notice kept"]);
  });

  it("follows the core with the first page that goes live only", () => {
    const gate = new CoreGate();
    const first = record();
    const second = record();
    gate.goLive(first.follow, first.answer);
    gate.goLive(second.follow, second.answer);
    gate.notice(said("once"));
    expect(first.seen).toEqual(["notice once"]);
    expect(second.seen).toEqual([]);
  });

  it("follows and answers nothing once closed, kept or new", () => {
    const gate = new CoreGate();
    const page = record();
    gate.notice(said("kept"));
    gate.request("held");
    gate.close();
    gate.goLive(page.follow, page.answer);
    gate.notice(said("after"));
    gate.request("after");
    expect(page.seen).toEqual([]);

    const live = new CoreGate();
    const followed = record();
    live.goLive(followed.follow, followed.answer);
    live.close();
    live.notice(said("after"));
    live.request("after");
    expect(followed.seen).toEqual([]);
  });
});
