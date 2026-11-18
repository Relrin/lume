import { describe, expect, it } from "vitest";

import { exitActivity, greatest, nextActivity, raise } from "../src/events/activity.ts";
import { applyEvent, PaneReports, type PaneReport } from "../src/events/store.ts";
import { STATUS_RECORDS_MAX } from "../src/limits.ts";
import type { ProgramState, SessionEvent } from "../src/transport/types.ts";

const EMPTY: PaneReport = { session: "s1", statuses: [], activity: "idle" };

function fold(events: readonly SessionEvent[], from: PaneReport = EMPTY): PaneReport {
  return events.reduce(applyEvent, from);
}

function status(state: ProgramState, id?: string, more: object = {}): SessionEvent {
  return { type: "program_status", state, ...(id === undefined ? {} : { id }), ...more };
}

function ids(report: PaneReport): (string | undefined)[] {
  return report.statuses.map((record) => record.id);
}

describe("folding a pane's reports, as the core folds them", () => {
  it("holds the latest report of each kind", () => {
    let report = fold([
      { type: "title", title: "one" },
      { type: "bell" },
      { type: "activity" },
      { type: "title", title: "two" },
      { type: "command_started", command: "make" },
    ]);
    expect(report.title).toBe("two");
    expect(report.command).toEqual({ state: "running", command: "make" });
    report = applyEvent(report, { type: "command_finished", exit: 1 });
    expect(report.command).toEqual({ state: "finished", exit: 1 });
    // A title set to nothing is no title.
    report = applyEvent(report, { type: "title", title: "" });
    expect(report.title).toBeUndefined();
    expect("title" in report).toBe(false);
  });

  it("replaces a status record whole and clears a record with those beneath it", () => {
    let report = fold([
      status("working"),
      status("working", "build"),
      status("working", "build/test"),
      status("working", "builder"),
      // A record reported again moves behind the others and holds only the new report.
      status("done", undefined, { app: "make" }),
    ]);
    expect(ids(report)).toEqual(["build", "build/test", "builder", undefined]);
    report = applyEvent(report, status("idle"));
    expect(report.statuses.at(-1)?.app).toBeUndefined();

    report = applyEvent(report, status("clear", "build"));
    expect(ids(report)).toEqual(["builder", undefined]);
    report = applyEvent(report, status("clear"));
    expect(report.statuses).toEqual([]);
  });

  it("keeps a limited number of records, and the oldest goes first", () => {
    const events = Array.from({ length: STATUS_RECORDS_MAX + 3 }, (_, n) =>
      status("working", `job${n}`),
    );
    const report = fold(events);
    expect(report.statuses).toHaveLength(STATUS_RECORDS_MAX);
    expect(report.statuses[0]?.id).toBe("job3");
  });

  it("takes a state in one piece in place of what was held", () => {
    const held = fold([
      { type: "title", title: "old" },
      { type: "foreground", pid: 7, name: "vim" },
    ]);
    const report = applyEvent(held, { type: "state", state: { cwd: "file://box/srv" } });
    expect(report).toEqual({
      session: "s1",
      statuses: [],
      activity: "idle",
      cwd: "file://box/srv",
    });
  });

  it("keeps what the page knows of the pane through a state in one piece", () => {
    const held: PaneReport = {
      session: "s1",
      launcher: { type: "wsl", distro: "Ubuntu" },
      profile: "auto.wsl.Ubuntu",
      statuses: [],
      exit: { code: 2 },
      activity: "attention",
    };
    const report = applyEvent(held, { type: "state", state: {} });
    expect(report.launcher).toEqual(held.launcher);
    expect(report.profile).toBe("auto.wsl.Ubuntu");
    expect(report.exit).toEqual({ code: 2 });
    expect(report.activity).toBe("attention");
  });

  it("makes every string safe to show", () => {
    const sneaky = "a\u001b[31mb‮c\u0007";
    const report = fold([
      { type: "title", title: sneaky },
      { type: "command_started", command: sneaky },
      { type: "foreground", pid: 1, name: sneaky },
      status("blocked", "x", { app: sneaky, title: sneaky, message: sneaky }),
      { type: "disconnected", reason: "failed", detail: sneaky },
    ]);
    const safe = "a[31mbc";
    expect(report.title).toBe(safe);
    expect(report.command).toEqual({ state: "running", command: safe });
    expect(report.foreground?.name).toBe(safe);
    expect(report.statuses[0]).toMatchObject({ app: safe, title: safe, message: safe });
    expect(report.disconnected?.detail).toBe(safe);
  });
});

describe("what a pane the user is not looking at gathers", () => {
  const before = { statuses: [] };

  it("never lowers what was gathered, and the error stands above the rest", () => {
    expect(raise("attention", "output")).toBe("attention");
    expect(raise("output", "attention-error")).toBe("attention-error");
    expect(greatest(["idle", "output", "attention"])).toBe("attention");
    expect(greatest([])).toBe("idle");
  });

  it("takes output, a bell, a command's end and a program that waits as news", () => {
    expect(nextActivity("idle", { type: "activity" }, before, false)).toBe("output");
    expect(nextActivity("idle", { type: "bell" }, before, false)).toBe("attention");
    expect(nextActivity("idle", { type: "command_finished", exit: 0 }, before, false)).toBe(
      "attention",
    );
    expect(nextActivity("idle", { type: "command_finished" }, before, false)).toBe("attention");
    expect(nextActivity("output", { type: "command_finished", exit: 3 }, before, false)).toBe(
      "attention-error",
    );
    expect(nextActivity("idle", status("blocked", "a"), before, false)).toBe("attention");
    expect(nextActivity("idle", status("error"), before, false)).toBe("attention-error");
    expect(nextActivity("idle", status("working"), before, false)).toBe("idle");
  });

  it("takes a program that already waited as no news", () => {
    const waiting = { statuses: [{ id: "a", state: "blocked" }] };
    expect(nextActivity("idle", status("blocked", "a"), waiting, false)).toBe("idle");
  });

  it("gathers nothing from a state in one piece", () => {
    const whole: SessionEvent = {
      type: "state",
      state: { command: { state: "finished", exit: 9 } },
    };
    expect(nextActivity("idle", whole, before, false)).toBe("idle");
  });

  it("gathers nothing while the user is looking", () => {
    expect(nextActivity("idle", { type: "bell" }, before, true)).toBe("idle");
    expect(exitActivity("idle", { code: 1 }, true)).toBe("idle");
  });

  it("takes the end of a program as news, and a failed end as an error", () => {
    expect(exitActivity("idle", { code: 0 }, false)).toBe("attention");
    expect(exitActivity("idle", { code: 1 }, false)).toBe("attention-error");
    expect(exitActivity("output", "unknown", false)).toBe("attention-error");
  });
});

describe("the reports of a window's panes", () => {
  function reports(looking: Set<string> = new Set()) {
    const store = new PaneReports((pane) => looking.has(pane));
    const told: string[] = [];
    store.subscribeAll((pane) => told.push(pane));
    return { store, told, looking };
  }

  it("begins afresh when a pane shows another session, with what the core said of it", () => {
    const { store } = reports();
    store.attached("p1", { session: "a", profile: "box" });
    store.reported("p1", { type: "title", title: "vim" });
    store.reported("p1", { type: "bell" });
    expect(store.get("p1")).toMatchObject({ session: "a", title: "vim", activity: "attention" });
    store.attached("p1", { session: "b", launcher: { type: "wsl", distro: "Ubuntu" } });
    expect(store.get("p1")).toEqual({
      session: "b",
      launcher: { type: "wsl", distro: "Ubuntu" },
      statuses: [],
      activity: "idle",
    });
    store.attached("p1", null);
    expect(store.get("p1").session).toBeNull();
  });

  it("tells whoever follows a pane of each change, and nobody of what changed nothing", () => {
    const { store, told } = reports();
    let heard = 0;
    const stop = store.subscribe("p1", () => (heard += 1));
    store.attached("p1", { session: "a" });
    store.reported("p1", { type: "bell" });
    // A second bell changes nothing that is held.
    store.reported("p1", { type: "bell" });
    store.reported("p2", { type: "bell" });
    expect(heard).toBe(2);
    expect(told).toEqual(["p1", "p1", "p2"]);
    stop();
    store.reported("p1", { type: "title", title: "x" });
    expect(heard).toBe(2);
  });

  it("clears what a pane gathered once the user looks at it", () => {
    const { store, looking } = reports();
    store.attached("p1", { session: "a" });
    store.reported("p1", { type: "activity" });
    store.exited("p1", { code: 4 });
    expect(store.get("p1").activity).toBe("attention-error");
    expect(store.get("p1").exit).toEqual({ code: 4 });
    looking.add("p1");
    store.attend("p1");
    expect(store.get("p1").activity).toBe("idle");
    store.reported("p1", { type: "bell" });
    expect(store.get("p1").activity).toBe("idle");
  });

  it("notes when another view sizes a pane's terminal, and keeps it through a whole state", () => {
    const { store, told } = reports();
    store.attached("p1", { session: "a" });
    store.sizedElsewhere("p1", true);
    store.sizedElsewhere("p1", true);
    expect(store.get("p1").sizedElsewhere).toBe(true);
    store.reported("p1", { type: "state", state: { title: "vim" } });
    expect(store.get("p1")).toMatchObject({ title: "vim", sizedElsewhere: true });
    store.sizedElsewhere("p1", false);
    expect("sizedElsewhere" in store.get("p1")).toBe(false);
    expect(told).toEqual(["p1", "p1", "p1", "p1"]);
    // A view that shows another session starts with its own size.
    store.sizedElsewhere("p1", true);
    store.attached("p1", { session: "b" });
    expect(store.get("p1").sizedElsewhere).toBeUndefined();
  });

  it("forgets a pane, and every pane", () => {
    const { store, told } = reports();
    store.attached("p1", { session: "a" });
    store.attached("p2", { session: "b" });
    store.forget("p1");
    expect(store.get("p1").session).toBeNull();
    store.clear();
    expect(store.get("p2").session).toBeNull();
    expect(told).toEqual(["p1", "p2", "p1", "p2"]);
  });
});
