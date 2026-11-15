import { describe, expect, it } from "vitest";

import type { Intent } from "../src/layout/types.ts";
import {
  answerRequest,
  type Match,
  type PaneRef,
  type RemoteTarget,
  type Screen,
} from "../src/terminal/remote.ts";
import { CommandError, type TermSize } from "../src/transport/types.ts";

const SESSION = "00000000000000ab";
const PANE = "p3";

/** A target that records what it was asked and answers from a script. */
class FakeTarget implements RemoteTarget {
  readonly calls: unknown[] = [];
  fail: CommandError | undefined;

  estimate(intent: Intent): TermSize {
    this.calls.push(["estimate", intent]);
    if (this.fail) throw this.fail;
    return { cols: 57, rows: 19 };
  }

  command(id: string, args: unknown): Promise<void> {
    this.calls.push(["command", id, args]);
    return this.fail ? Promise.reject(this.fail) : Promise.resolve();
  }

  screen(target: PaneRef): Screen {
    this.calls.push(["screen", target]);
    if (this.fail) throw this.fail;
    return {
      pane: PANE,
      session: SESSION,
      size: { cols: 4, rows: 2 },
      cursor: { col: 2, row: 1, visible: true },
      alternate: false,
      lines: ["ab", "$ "],
    };
  }

  waitText(target: PaneRef, text: string, timeoutMs: number): Promise<Match> {
    this.calls.push(["waitText", target, text, timeoutMs]);
    if (this.fail) return Promise.reject(this.fail);
    return Promise.resolve({ pane: PANE, session: SESSION, row: 1, line: "$ " });
  }

  resizeTo(target: PaneRef, size: TermSize): Promise<TermSize> {
    this.calls.push(["resizeTo", target, size]);
    if (this.fail) return Promise.reject(this.fail);
    return Promise.resolve(size);
  }
}

function request(id: number, body: unknown): unknown {
  return { request: { id, body } };
}

describe("a request from the core", () => {
  it("says how large the pane an intent would make will be", async () => {
    const target = new FakeTarget();
    const intent = { type: "tab_new", window: "w1", content: { type: "terminal" } };
    const answer = await answerRequest(target, request(1, { type: "estimate", intent }));
    expect(answer).toEqual({
      id: 1,
      result: { ok: { type: "estimated", size: { cols: 57, rows: 19 } } },
    });
    expect(target.calls).toEqual([["estimate", intent]]);
  });

  it("runs a command with the arguments it was given, or with none", async () => {
    const target = new FakeTarget();
    const answer = await answerRequest(
      target,
      request(2, { type: "command", id: "lume.terminal.copy", args: { pane: PANE } }),
    );
    expect(answer).toEqual({
      id: 2,
      result: { ok: { type: "ran", command: "lume.terminal.copy" } },
    });
    await answerRequest(target, request(3, { type: "command", id: "lume.view.zoom-in" }));
    expect(target.calls).toEqual([
      ["command", "lume.terminal.copy", { pane: PANE }],
      ["command", "lume.view.zoom-in", null],
    ]);
  });

  it("reads the screen of the focused pane, or of the pane or session it names", async () => {
    const target = new FakeTarget();
    const answer = await answerRequest(target, request(3, { type: "screen" }));
    expect(answer).toEqual({
      id: 3,
      result: {
        ok: {
          type: "screen",
          pane: PANE,
          session: SESSION,
          size: { cols: 4, rows: 2 },
          cursor: { col: 2, row: 1, visible: true },
          alternate: false,
          lines: ["ab", "$ "],
        },
      },
    });
    await answerRequest(target, request(4, { type: "screen", pane: PANE }));
    await answerRequest(target, request(5, { type: "screen", session: SESSION }));
    expect(target.calls).toEqual([
      ["screen", {}],
      ["screen", { pane: PANE }],
      ["screen", { session: SESSION }],
    ]);
  });

  it("waits for text for as long as it says", async () => {
    const target = new FakeTarget();
    const answer = await answerRequest(
      target,
      request(5, { type: "wait_text", pane: PANE, text: "$ ", timeout_ms: 250 }),
    );
    expect(answer).toEqual({
      id: 5,
      result: { ok: { type: "matched", pane: PANE, session: SESSION, row: 1, line: "$ " } },
    });
    expect(target.calls).toEqual([["waitText", { pane: PANE }, "$ ", 250]]);
  });

  it("gives the terminal the size it asks for", async () => {
    const target = new FakeTarget();
    const answer = await answerRequest(
      target,
      request(6, { type: "resize", pane: PANE, size: { cols: 90, rows: 20 } }),
    );
    expect(answer).toEqual({
      id: 6,
      result: { ok: { type: "resized", size: { cols: 90, rows: 20 } } },
    });
    expect(target.calls).toEqual([["resizeTo", { pane: PANE }, { cols: 90, rows: 20 }]]);
  });

  it("is told why it failed, with a code to match on", async () => {
    const target = new FakeTarget();
    target.fail = new CommandError("not_shown", "this window has no such pane");
    for (const body of [
      { type: "estimate", intent: { type: "tab_new" } },
      { type: "command", id: "lume.terminal.copy" },
      { type: "screen" },
      { type: "wait_text", text: "x", timeout_ms: 1 },
      { type: "resize", size: { cols: 80, rows: 24 } },
    ]) {
      const answer = await answerRequest(target, request(7, body));
      expect(answer).toEqual({
        id: 7,
        result: { err: { code: "not_shown", message: "this window has no such pane" } },
      });
    }
  });

  it("is refused when it is not one the page knows or is not well formed", async () => {
    const target = new FakeTarget();
    const refused = async (body: unknown) => {
      const answer = await answerRequest(target, request(8, body));
      expect(answer?.result).toMatchObject({ err: { code: "bad_request" } });
    };
    await refused({ type: "dance" });
    await refused(undefined);
    await refused({ type: "open", spec: {} });
    await refused({ type: "estimate" });
    await refused({ type: "estimate", intent: { pane: "p1" } });
    await refused({ type: "command" });
    await refused({ type: "command", id: "" });
    await refused({ type: "screen", session: 7 });
    await refused({ type: "screen", pane: 3 });
    await refused({ type: "wait_text", text: "", timeout_ms: 5 });
    await refused({ type: "wait_text", text: "x" });
    await refused({ type: "resize", size: { cols: 0, rows: 24 } });
    await refused({ type: "resize" });
    expect(target.calls).toEqual([]);
  });

  it("cannot be answered when it has no id", async () => {
    const target = new FakeTarget();
    expect(await answerRequest(target, { notify: { type: "notice" } })).toBeUndefined();
    expect(await answerRequest(target, { request: { body: { type: "screen" } } })).toBeUndefined();
    expect(await answerRequest(target, "screen")).toBeUndefined();
    expect(target.calls).toEqual([]);
  });

  it("reports a fault of the page as its own", async () => {
    const target = new FakeTarget();
    target.screen = () => {
      throw new Error("the terminal is gone");
    };
    const answer = await answerRequest(target, request(9, { type: "screen" }));
    expect(answer).toEqual({
      id: 9,
      result: { err: { code: "internal", message: "the terminal is gone" } },
    });
  });
});
