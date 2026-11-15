import { describe, expect, it } from "vitest";

import {
  answerRequest,
  type Match,
  type Opened,
  type OpenSpec,
  type RemoteTarget,
  type Screen,
} from "../src/terminal/remote.ts";
import { CommandError, type TermSize } from "../src/transport/types.ts";

const SESSION = "00000000000000ab";

/** A target that records what it was asked and answers from a script. */
class FakeTarget implements RemoteTarget {
  readonly calls: unknown[] = [];
  fail: CommandError | undefined;

  open(spec: OpenSpec): Promise<Opened> {
    this.calls.push(["open", spec]);
    if (this.fail) return Promise.reject(this.fail);
    return Promise.resolve({ session: SESSION, previous: null });
  }

  screen(session: string | undefined): Screen {
    this.calls.push(["screen", session]);
    if (this.fail) throw this.fail;
    return {
      session: SESSION,
      size: { cols: 4, rows: 2 },
      cursor: { col: 2, row: 1, visible: true },
      alternate: false,
      lines: ["ab", "$ "],
    };
  }

  waitText(session: string | undefined, text: string, timeoutMs: number): Promise<Match> {
    this.calls.push(["waitText", session, text, timeoutMs]);
    if (this.fail) return Promise.reject(this.fail);
    return Promise.resolve({ session: SESSION, row: 1, line: "$ " });
  }

  resizeTo(session: string | undefined, size: TermSize): Promise<TermSize> {
    this.calls.push(["resizeTo", session, size]);
    if (this.fail) return Promise.reject(this.fail);
    return Promise.resolve(size);
  }
}

function request(id: number, body: unknown): unknown {
  return { request: { id, body } };
}

describe("a request from the core", () => {
  it("starts a session with what it chose and nothing more", async () => {
    const target = new FakeTarget();
    const answer = await answerRequest(
      target,
      request(1, {
        type: "open",
        spec: {
          launcher: { type: "shell", program: "cmd.exe", args: ["/d"], login: false },
          cwd: "C:\\",
          env: [["A", "1"]],
          size: { cols: 100, rows: 30 },
        },
      }),
    );
    expect(answer).toEqual({
      id: 1,
      result: { ok: { type: "spawned", session: SESSION, previous: null } },
    });
    expect(target.calls).toEqual([
      [
        "open",
        {
          launcher: { type: "shell", program: "cmd.exe", args: ["/d"], login: false },
          cwd: "C:\\",
          env: [["A", "1"]],
          size: { cols: 100, rows: 30 },
        },
      ],
    ]);

    await answerRequest(target, request(2, { type: "open", spec: {} }));
    expect(target.calls[1]).toEqual(["open", {}]);
  });

  it("reads the screen of the session shown, or of the one it names", async () => {
    const target = new FakeTarget();
    const answer = await answerRequest(target, request(3, { type: "screen" }));
    expect(answer).toEqual({
      id: 3,
      result: {
        ok: {
          type: "screen",
          session: SESSION,
          size: { cols: 4, rows: 2 },
          cursor: { col: 2, row: 1, visible: true },
          alternate: false,
          lines: ["ab", "$ "],
        },
      },
    });
    await answerRequest(target, request(4, { type: "screen", session: SESSION }));
    expect(target.calls).toEqual([
      ["screen", undefined],
      ["screen", SESSION],
    ]);
  });

  it("waits for text for as long as it says", async () => {
    const target = new FakeTarget();
    const answer = await answerRequest(
      target,
      request(5, { type: "wait_text", text: "$ ", timeout_ms: 250 }),
    );
    expect(answer).toEqual({
      id: 5,
      result: { ok: { type: "matched", session: SESSION, row: 1, line: "$ " } },
    });
    expect(target.calls).toEqual([["waitText", undefined, "$ ", 250]]);
  });

  it("gives the terminal the size it asks for", async () => {
    const target = new FakeTarget();
    const answer = await answerRequest(
      target,
      request(6, { type: "resize", session: SESSION, size: { cols: 90, rows: 20 } }),
    );
    expect(answer).toEqual({
      id: 6,
      result: { ok: { type: "resized", size: { cols: 90, rows: 20 } } },
    });
    expect(target.calls).toEqual([["resizeTo", SESSION, { cols: 90, rows: 20 }]]);
  });

  it("is told why it failed, with a code to match on", async () => {
    const target = new FakeTarget();
    target.fail = new CommandError("not_shown", "the window shows another session");
    for (const body of [
      { type: "open", spec: {} },
      { type: "screen" },
      { type: "wait_text", text: "x", timeout_ms: 1 },
      { type: "resize", size: { cols: 80, rows: 24 } },
    ]) {
      const answer = await answerRequest(target, request(7, body));
      expect(answer).toEqual({
        id: 7,
        result: { err: { code: "not_shown", message: "the window shows another session" } },
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
    await refused({ type: "open" });
    await refused({ type: "screen", session: 7 });
    await refused({ type: "wait_text", text: "", timeout_ms: 5 });
    await refused({ type: "wait_text", text: "x" });
    await refused({ type: "resize", size: { cols: 0, rows: 24 } });
    await refused({ type: "resize" });
    await refused({ type: "open", spec: { size: { cols: 80 } } });
    expect(target.calls).toEqual([]);
  });

  it("cannot be answered when it has no id", async () => {
    const target = new FakeTarget();
    expect(await answerRequest(target, { notify: { type: "fatal" } })).toBeUndefined();
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
