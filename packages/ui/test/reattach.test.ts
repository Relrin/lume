import { describe, expect, it } from "vitest";

import { reattachOrder } from "../src/terminal/reattach.ts";
import type { ExitStatus, SessionInfo } from "../src/transport/types.ts";

function session(id: string, exit: ExitStatus | null = null): SessionInfo {
  return {
    backend: "local",
    id,
    program: "sh",
    pid: 1,
    size: { cols: 80, rows: 24 },
    createdUnixMs: 0,
    attached: false,
    exit,
  };
}

const ids = (sessions: readonly SessionInfo[]) => sessions.map((entry) => entry.id);

describe("the order a reloaded page tries its sessions in", () => {
  it("starts with the session the page showed", () => {
    const sessions = [session("a"), session("b"), session("c")];
    expect(ids(reattachOrder(sessions, "b"))).toEqual(["b", "a", "c"]);
  });

  it("is the listed order when the page remembers nothing, or a session that is gone", () => {
    const sessions = [session("a"), session("b")];
    expect(ids(reattachOrder(sessions, null))).toEqual(["a", "b"]);
    expect(ids(reattachOrder(sessions, "z"))).toEqual(["a", "b"]);
  });

  it("leaves out sessions whose program has ended", () => {
    const sessions = [session("a", { code: 0 }), session("b"), session("c", "unknown")];
    expect(ids(reattachOrder(sessions, "a"))).toEqual(["b"]);
    expect(reattachOrder([], "a")).toEqual([]);
  });
});
