import { describe, expect, it } from "vitest";

import { EventError, sessionEvent } from "../src/transport/events.ts";

describe("what a session reports", () => {
  it("is read as it was sent, without the fields it does not have", () => {
    const reports: unknown[] = [
      { type: "title", title: "vim" },
      { type: "cwd", url: "file://box/home/me" },
      { type: "bell" },
      { type: "activity" },
      { type: "command_started" },
      { type: "command_started", command: "make test" },
      { type: "command_finished" },
      { type: "command_finished", exit: -1 },
      { type: "program_status", state: "clear" },
      {
        type: "program_status",
        state: "blocked",
        id: "deploy/us-east",
        kind: "permission",
        progress: 40,
        app: "terraform",
        title: "Plan",
        message: "Apply 3 resources?",
      },
      { type: "foreground", pid: 4242, name: "vim" },
      { type: "disconnected", reason: "exited" },
      { type: "disconnected", reason: "connection_lost", detail: "Broken pipe" },
      { type: "state", state: {} },
      {
        type: "state",
        state: {
          title: "~/src",
          cwd: "file://box/src",
          command: { state: "running", command: "vim notes.md" },
          statuses: [{ state: "done" }, { state: "working", id: "build", progress: 0 }],
          foreground: { pid: 7, name: "vim" },
          disconnected: { reason: "closed" },
        },
      },
      { type: "state", state: { command: { state: "finished", exit: 2 } } },
      { type: "state", state: { command: { state: "finished" } } },
    ];
    for (const report of reports) {
      expect(sessionEvent(structuredClone(report))).toEqual(report);
    }
  });

  it("is known by its name alone when it carries nothing", () => {
    expect(sessionEvent({ type: "bell", loud: true })).toEqual({ type: "bell" });
  });

  it("is refused when it is not one of the reports there are", () => {
    const refused: unknown[] = [
      null,
      "bell",
      [],
      {},
      { type: "mystery" },
      { type: 7 },
      // A field that is missing, of the wrong kind, or one too many.
      { type: "title" },
      { type: "title", title: 7 },
      { type: "title", title: "vim", more: 1 },
      { type: "cwd", path: "/home/me" },
      { type: "command_started", command: 7 },
      { type: "command_finished", exit: 1.5 },
      { type: "command_finished", exit: "1" },
      { type: "foreground", pid: 7 },
      { type: "foreground", pid: -1, name: "vim" },
      { type: "foreground", pid: "7", name: "vim" },
      // A choice that is not among the choices.
      { type: "program_status", state: "paused" },
      { type: "program_status", state: "blocked", kind: "mood" },
      { type: "program_status", state: "working", progress: 101 },
      { type: "program_status" },
      { type: "disconnected", reason: "bored" },
      { type: "disconnected" },
      // A state that is not a state, or has in it what is not one of its parts.
      { type: "state" },
      { type: "state", state: [] },
      { type: "state", state: { colour: "red" } },
      { type: "state", state: { statuses: {} } },
      { type: "state", state: { statuses: [{ state: "done", more: 1 }] } },
      { type: "state", state: { command: { state: "paused" } } },
      { type: "state", state: { command: { state: "running", exit: 1 } } },
      { type: "state", state: { foreground: { pid: 7, name: "vim", more: 1 } } },
      { type: "state", state: { disconnected: { reason: "exited", when: 1 } } },
    ];
    for (const report of refused) {
      expect(() => sessionEvent(report), JSON.stringify(report)).toThrow(EventError);
    }
  });
});
