import { describe, expect, it } from "vitest";

import {
  SearchController,
  searchLabel,
  type PaneSearch,
  type SearchTarget,
} from "../src/search/controller.ts";
import type { SearchDirection, SearchProgress } from "../src/terminal/view.ts";
import {
  SEARCH_QUERY_DEBOUNCE_MS,
  SEARCH_REFEED_INTERVAL_MS,
  SEARCH_SLICE_MS,
} from "../src/tuning.ts";

/**
 * A terminal that is searched by counting: every stretch finds `perPump` more of the
 * `matches` there are, and takes `sliceMs` of the clock to do it.
 */
class FakeTarget implements SearchTarget {
  needle = "";
  matches = 0;
  perPump = 10;
  sliceMs = 1;
  cap = Infinity;
  held = false;
  found = 0;
  selected: number | undefined;
  readonly log: string[] = [];
  readonly #clock: { now: number };

  constructor(clock: { now: number }) {
    this.#clock = clock;
  }

  search(text: string): void {
    this.log.push(`search ${JSON.stringify(text)}`);
    this.needle = text;
    this.found = 0;
    this.selected = undefined;
  }

  searchStep(direction: SearchDirection): void {
    this.log.push(`step ${direction}`);
    if (this.found === 0) return;
    const step = direction === "older" ? 1 : -1;
    this.selected = ((this.selected ?? -step) + step + this.found) % this.found;
  }

  searchPump(budgetMs: number): SearchProgress {
    this.log.push(`pump ${budgetMs}`);
    if (!this.held && this.needle !== "") {
      this.#clock.now += this.sliceMs;
      this.found = Math.min(this.matches, this.found + this.perPump);
      if (this.found > 0) this.selected ??= 0;
    }
    return this.searchProgress();
  }

  searchProgress(): SearchProgress {
    if (this.needle === "") return { status: "idle", total: 0, selected: undefined, capped: false };
    const capped = this.found >= this.cap;
    return {
      status: this.held ? "held" : this.found < this.matches ? "running" : "complete",
      total: this.found,
      selected: this.selected,
      capped,
    };
  }
}

function setup(panes: readonly string[] = ["p1"]) {
  const clock = { now: 0 };
  const targets = new Map(panes.map((pane) => [pane, new FakeTarget(clock)]));
  const soon: (() => void)[] = [];
  const timers = new Map<number, { at: number; task: () => void }>();
  let nextTimer = 1;
  const controller = new SearchController({
    target: (pane) => targets.get(pane),
    soon: (task) => void soon.push(task),
    setTimer: (task, ms) => {
      timers.set(nextTimer, { at: clock.now + ms, task });
      return nextTimer++;
    },
    clearTimer: (timer) => void timers.delete(timer),
    now: () => clock.now,
  });
  /** Runs what was put off until soon, and what that puts off, until nothing is. */
  const settle = (turns = 1000): number => {
    let ran = 0;
    while (soon.length > 0 && ran < turns) {
      soon.shift()!();
      ran++;
    }
    return ran;
  };
  /** Lets `ms` pass and runs the timers that come due, in order. */
  const pass = (ms: number): void => {
    const until = clock.now + ms;
    for (;;) {
      const due = [...timers.entries()]
        .filter(([, timer]) => timer.at <= until)
        .sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      timers.delete(due[0]);
      clock.now = Math.max(clock.now, due[1].at);
      due[1].task();
    }
    clock.now = until;
  };
  const target = (pane = "p1") => targets.get(pane)!;
  return { controller, clock, targets, target, settle, pass, soon, timers };
}

describe("what a search bar says", () => {
  const search = (change: Partial<PaneSearch>): PaneSearch => ({
    open: true,
    query: "x",
    keyboard: "bar",
    total: 0,
    selected: undefined,
    running: false,
    capped: false,
    ...change,
  });

  it("says nothing while nothing is looked for", () => {
    expect(searchLabel(search({ query: "", total: 4 }))).toBe("");
  });

  it("says which match of how many", () => {
    expect(searchLabel(search({ total: 17, selected: 0 }))).toBe("1 of 17");
    expect(searchLabel(search({ total: 17, selected: 16 }))).toBe("17 of 17");
    expect(searchLabel(search({ total: 17 }))).toBe("17");
  });

  it("says when there are more than it counts", () => {
    expect(searchLabel(search({ total: 10_000, selected: 2, capped: true }))).toBe("3 of 10000+");
  });

  it("tells looking from not finding", () => {
    expect(searchLabel(search({ running: true }))).toBe("Searching");
    expect(searchLabel(search({}))).toBe("No results");
  });
});

describe("searching a pane", () => {
  it("is closed until it is opened, and looks for nothing until something is typed", () => {
    const { controller, target, settle } = setup();
    expect(controller.of("p1").open).toBe(false);
    controller.open("p1");
    expect(controller.of("p1")).toMatchObject({ open: true, query: "", keyboard: "bar" });
    expect(settle()).toBe(0);
    expect(target().log).toEqual(['search ""']);
  });

  it("waits for a pause in the typing before it looks", () => {
    const { controller, target, pass, settle } = setup();
    target().matches = 3;
    controller.open("p1");
    controller.setQuery("p1", "e");
    controller.setQuery("p1", "er");
    controller.setQuery("p1", "err");
    pass(SEARCH_QUERY_DEBOUNCE_MS - 1);
    expect(target().needle).toBe("");
    pass(1);
    expect(target().needle).toBe("err");
    settle();
    expect(controller.of("p1")).toMatchObject({ total: 3, selected: 0, running: false });
  });

  it("looks at once for a text it is opened with", () => {
    const { controller, target, settle } = setup();
    target().matches = 2;
    controller.open("p1", "needle");
    expect(target().needle).toBe("needle");
    expect(controller.of("p1").running).toBe(true);
    settle();
    expect(controller.of("p1")).toMatchObject({ query: "needle", total: 2, running: false });
  });

  it("stops showing matches the moment the field is emptied", () => {
    const { controller, target, settle } = setup();
    target().matches = 2;
    controller.open("p1", "needle");
    settle();
    controller.setQuery("p1", "");
    expect(target().needle).toBe("");
    expect(controller.of("p1")).toMatchObject({ total: 0, selected: undefined, running: false });
  });

  it("looks in stretches, one for each turn it is given", () => {
    const { controller, target, settle, soon } = setup();
    target().matches = 35;
    controller.open("p1", "x");
    expect(soon).toHaveLength(1);
    expect(settle(1)).toBe(1);
    expect(controller.of("p1")).toMatchObject({ total: 10, running: true });
    expect(soon).toHaveLength(1);
    settle();
    expect(controller.of("p1")).toMatchObject({ total: 35, running: false });
    expect(target().log.filter((line) => line.startsWith("pump"))).toEqual(
      Array.from({ length: 4 }, () => `pump ${SEARCH_SLICE_MS}`),
    );
  });

  it("gives panes turns when several are searched", () => {
    const { controller, target, settle } = setup(["p1", "p2"]);
    target("p1").matches = 30;
    target("p2").matches = 30;
    controller.open("p1", "x");
    controller.open("p2", "x");
    settle(2);
    expect(controller.of("p1").total).toBe(10);
    expect(controller.of("p2").total).toBe(10);
    settle();
    expect(controller.of("p1").total).toBe(30);
    expect(controller.of("p2").total).toBe(30);
  });

  it("stops looking once the terminal says it counts no further", () => {
    const { controller, target, settle } = setup();
    target().matches = 1000;
    target().cap = 20;
    controller.open("p1", "x");
    settle();
    expect(controller.of("p1")).toMatchObject({ total: 20, capped: true, running: false });
  });

  it("measures how long a search took and its longest stretch", () => {
    const { controller, target, settle } = setup();
    target().matches = 30;
    target().sliceMs = 7;
    controller.open("p1", "x");
    expect(controller.lastRun("p1")).toEqual({ completeMs: undefined, maxSliceMs: 0 });
    settle();
    expect(controller.lastRun("p1")).toEqual({ completeMs: 21, maxSliceMs: 7 });
  });

  it("steps through the matches", () => {
    const { controller, target, settle } = setup();
    target().matches = 3;
    controller.open("p1", "x");
    settle();
    controller.step("p1", "older");
    expect(controller.of("p1").selected).toBe(1);
    controller.step("p1", "newer");
    controller.step("p1", "newer");
    expect(controller.of("p1").selected).toBe(2);
  });

  it("looks for what was just typed when asked to step, without waiting", () => {
    const { controller, target, settle } = setup();
    target().matches = 3;
    controller.open("p1");
    controller.setQuery("p1", "abc");
    controller.step("p1", "older");
    expect(target().needle).toBe("abc");
    expect(target().log).not.toContain("step older");
    settle();
    expect(controller.of("p1").selected).toBe(0);
  });

  it("ends the search when its bar is put away, and remembers what was typed", () => {
    const { controller, target, settle } = setup();
    target().matches = 100;
    controller.open("p1", "x");
    settle(2);
    controller.close("p1");
    expect(target().needle).toBe("");
    expect(settle()).toBeLessThanOrEqual(1);
    expect(controller.of("p1")).toMatchObject({ open: false, query: "x", total: 0 });

    target().matches = 4;
    controller.open("p1");
    expect(target().needle).toBe("x");
    settle();
    expect(controller.of("p1").total).toBe(4);
  });
});

describe("a search and its terminal", () => {
  it("looks through what was printed since, but not more often than it should", () => {
    const { controller, target, settle, pass, soon, clock } = setup();
    target().matches = 5;
    controller.open("p1", "x");
    settle();
    expect(controller.of("p1").total).toBe(5);

    // The terminal printed, and says so while the search has only just ended.
    target().matches = 9;
    controller.touched("p1");
    expect(soon).toHaveLength(0);
    controller.touched("p1");
    pass(SEARCH_REFEED_INTERVAL_MS);
    settle();
    expect(controller.of("p1").total).toBe(9);

    // Later, with the search long ended, it is looked through at once.
    clock.now += 10 * SEARCH_REFEED_INTERVAL_MS;
    target().matches = 12;
    controller.touched("p1");
    expect(soon).toHaveLength(1);
    settle();
    expect(controller.of("p1").total).toBe(12);
  });

  it("shows the numbers a terminal reports without being asked", () => {
    const { controller, target, settle } = setup();
    target().matches = 5;
    controller.open("p1", "x");
    settle();
    target().selected = 3;
    controller.touched("p1");
    expect(controller.of("p1").selected).toBe(3);
  });

  it("waits while the terminal settles after being resized", () => {
    const { controller, target, settle } = setup();
    target().matches = 30;
    controller.open("p1", "x");
    settle(1);
    target().held = true;
    settle();
    expect(controller.of("p1").running).toBe(true);
    expect(controller.lastRun("p1")?.completeMs).toBeUndefined();
    target().held = false;
    controller.touched("p1");
    settle();
    expect(controller.of("p1")).toMatchObject({ total: 30, running: false });
  });

  it("is not told of a pane that was never searched", () => {
    const { controller } = setup();
    controller.touched("p1");
    expect(controller.store.getState().panes).toEqual({});
  });
});

describe("the keyboard and the panes", () => {
  it("gives the keyboard to a bar only where it was last, unless told to", () => {
    const { controller } = setup();
    let focused = 0;
    const forget = controller.registerBar("p1", { focus: () => focused++ });
    expect(controller.focusBar("p1")).toBe(false);
    controller.open("p1");
    expect(controller.focusBar("p1")).toBe(true);
    controller.setKeyboard("p1", "terminal");
    expect(controller.focusBar("p1")).toBe(false);
    expect(controller.focusBar("p1", true)).toBe(true);
    expect(focused).toBe(2);
    forget();
    expect(controller.focusBar("p1", true)).toBe(false);
  });

  it("forgets the searches of panes that are gone", () => {
    const { controller, target, settle, soon } = setup(["p1", "p2"]);
    target("p1").matches = 100;
    target("p2").matches = 100;
    controller.open("p1", "x");
    controller.open("p2", "y");
    settle(2);
    controller.prune(new Set(["p2"]));
    expect(Object.keys(controller.store.getState().panes)).toEqual(["p2"]);
    settle();
    expect(controller.of("p2").total).toBe(100);
    expect(target("p1").found).toBe(10);
    controller.reset();
    expect(controller.store.getState().panes).toEqual({});
    expect(soon).toHaveLength(0);
  });
});
