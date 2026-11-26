import { DEFAULT_TOKENS } from "@lume/tokens";
import { describe, expect, it } from "vitest";

import { createConfigStore } from "../src/config/store.ts";
import type { ConfigView, WindowAction } from "../src/core/types.ts";
import { createLayoutStore } from "../src/layout/store.ts";
import type { PaneId } from "../src/layout/types.ts";
import {
  closeRequested,
  closingText,
  confirmRequested,
  mayRun,
  panesClosedBy,
  type ClosingApp,
  type ConfirmingApp,
} from "../src/shell/closing.ts";
import { PromptStore, type Closing, type Confirmation } from "../src/terminal/prompts.ts";
import { oneTab, pane, split, viewOf } from "./layout-support.ts";
import { settings } from "./config-support.ts";

function view(confirm: "never" | "always"): ConfigView {
  return {
    revision: 1,
    platform: "windows",
    scheme: "dark",
    theme: { id: "lume-dark", scheme: "dark" },
    config: settings({
      appearance: { "pane-header": "always" },
      terminal: {
        "scrollback-lines": 10_000,
        "cursor-style": "block",
        "cursor-blink": true,
        "confirm-close": confirm,
      },
      layout: { "tab-placement": "top" },
      renderer: { tier: "auto" },
    }),
    sources: {},
    problems: [],
    tokens: DEFAULT_TOKENS,
    keymap: { bindings: [], removed: [] },
    schema_revision: 1,
    title_bar: "custom",
  };
}

/**
 * A window of one tab with two panes, `p1` having the keyboard, as far as closing sees
 * it. `ended` are the panes whose program has ended; `idle`, those whose shell the core
 * says is at its prompt; `fails`, whether the core cannot be asked.
 */
function page(
  confirm: "never" | "always",
  ended: readonly PaneId[] = [],
  idle: readonly PaneId[] = [],
  fails = false,
) {
  const layout = createLayoutStore();
  layout
    .getState()
    .reset(viewOf(oneTab(split("s1", "row", [500_000, 500_000], [pane("p1"), pane("p2")]), "p1")));
  const config = createConfigStore();
  config.getState().set(view(confirm));
  const prompts = new PromptStore();
  const asked: Closing[] = [];
  const closed: { action: WindowAction; confirmed: boolean }[] = [];
  let refocused = 0;
  /** Answers every question as soon as it is put, and remembers what was asked. */
  const answer = (ok: boolean) =>
    prompts.subscribe(() => {
      const current = prompts.current();
      if (current?.kind !== "close") return;
      asked.push(current.closing);
      current.answer(ok);
    });
  const app = {
    layout,
    config,
    prompts,
    views: {
      get: (id: PaneId) => (id === "p1" || id === "p2" ? { ended: ended.includes(id) } : undefined),
      focus: () => void (refocused += 1),
    },
    client: {
      windowControl: (action: WindowAction, confirmed = false) => {
        closed.push({ action, confirmed });
        return Promise.resolve();
      },
      panesBusy: (panes: readonly PaneId[]) =>
        fails
          ? Promise.reject(new Error("the core is gone"))
          : Promise.resolve(panes.filter((id) => !ended.includes(id) && !idle.includes(id))),
    },
  } as unknown as ClosingApp;
  return { app, answer, asked, closed, refocused: () => refocused };
}

describe("asking before closing what still runs a program", () => {
  it("works out which panes a command would close", () => {
    const state = page("always").app.layout.getState();
    expect(panesClosedBy(state, "lume.pane.close", null)).toEqual({ what: "pane", panes: ["p1"] });
    expect(panesClosedBy(state, "lume.pane.close", { pane: "p2" })).toEqual({
      what: "pane",
      panes: ["p2"],
    });
    expect(panesClosedBy(state, "lume.tab.close", null)).toEqual({
      what: "tab",
      panes: ["p1", "p2"],
    });
    expect(panesClosedBy(state, "lume.tab.close", { tab: "t1" })?.panes).toEqual(["p1", "p2"]);
    expect(panesClosedBy(state, "lume.tab.new", null)).toBeUndefined();
  });

  it("does not ask unless the configuration says to", async () => {
    const { app, answer, asked } = page("never");
    answer(false);
    expect(await mayRun(app, "lume.pane.close", null)).toBe(true);
    expect(asked).toEqual([]);
  });

  it("does not ask about a command that closes nothing", async () => {
    const { app, answer, asked } = page("always");
    answer(false);
    expect(await mayRun(app, "lume.pane.split-right", null)).toBe(true);
    expect(asked).toEqual([]);
  });

  it("asks about a pane whose program is running, and takes no for an answer", async () => {
    const { app, answer, asked, refocused } = page("always");
    answer(false);
    expect(await mayRun(app, "lume.pane.close", null)).toBe(false);
    expect(asked).toEqual([{ what: "pane", running: 1 }]);
    // The keyboard goes back to the terminal when nothing is closed.
    expect(refocused()).toBe(1);
  });

  it("goes ahead on a yes", async () => {
    const { app, answer, refocused } = page("always");
    answer(true);
    expect(await mayRun(app, "lume.tab.close", { tab: "t1" })).toBe(true);
    expect(refocused()).toBe(0);
  });

  it("counts only the panes whose programs have not ended", async () => {
    const some = page("always", ["p1"]);
    some.answer(true);
    await mayRun(some.app, "lume.tab.close", null);
    expect(some.asked).toEqual([{ what: "tab", running: 1 }]);

    const none = page("always", ["p1", "p2"]);
    none.answer(false);
    expect(await mayRun(none.app, "lume.tab.close", null)).toBe(true);
    expect(await mayRun(none.app, "lume.pane.close", { pane: "p2" })).toBe(true);
    expect(none.asked).toEqual([]);
  });

  it("does not ask about a shell the core says is at its prompt", async () => {
    const { app, answer, asked } = page("always", [], ["p1"]);
    answer(false);
    expect(await mayRun(app, "lume.pane.close", null)).toBe(true);
    expect(await mayRun(app, "lume.tab.close", null)).toBe(false);
    expect(asked).toEqual([{ what: "tab", running: 1 }]);
  });

  it("counts every pane whose program has not ended when the core cannot say", async () => {
    const { app, answer, asked } = page("always", [], ["p1", "p2"], true);
    answer(true);
    expect(await mayRun(app, "lume.tab.close", null)).toBe(true);
    expect(asked).toEqual([{ what: "tab", running: 2 }]);
  });

  it("names what is at work, what waits on the user first", async () => {
    const { app, answer, asked } = page("always");
    answer(false);
    const named: ClosingApp = {
      ...app,
      work: (id) => ({
        pane: id,
        tab: "t1",
        tabLabel: "t1",
        who: id === "p1" ? "cargo" : "claude",
        line: "",
        state: id === "p1" ? "work" : "wait",
        agent: id === "p2",
        unread: false,
      }),
    };
    expect(await mayRun(named, "lume.tab.close", null)).toBe(false);
    expect(asked[0]?.busy).toEqual([
      { pane: "p2", who: "claude", rest: "is waiting for your answer", waiting: true, agent: true },
      { pane: "p1", who: "cargo", rest: "is still running", waiting: false, agent: false },
    ]);
  });

  it("says a program the page has no word of is still running", async () => {
    const { app, answer, asked } = page("always");
    answer(false);
    await mayRun({ ...app, work: () => null }, "lume.pane.close", null);
    expect(asked[0]?.busy).toEqual([
      { pane: "p1", who: "A program", rest: "is still running", waiting: false, agent: false },
    ]);
  });

  it("closes a window it was asked about once the user agrees", async () => {
    const { app, answer, asked, closed } = page("never");
    answer(true);
    await closeRequested(app);
    expect(asked).toEqual([{ what: "window", running: 2 }]);
    expect(closed).toEqual([{ action: "close", confirmed: true }]);
  });

  it("leaves the window open on a no", async () => {
    const { app, answer, closed, refocused } = page("never");
    answer(false);
    await closeRequested(app);
    expect(closed).toEqual([]);
    expect(refocused()).toBe(1);
  });

  it("closes a window in which nothing runs without asking", async () => {
    const { app, answer, asked, closed } = page("never", ["p1", "p2"]);
    answer(false);
    await closeRequested(app);
    expect(asked).toEqual([]);
    expect(closed).toEqual([{ action: "close", confirmed: true }]);
  });

  it("says what closing would end", () => {
    expect(closingText({ what: "pane", running: 1 })).toBe(
      "It has a program that is still running. Closing the pane ends it.",
    );
    expect(closingText({ what: "tab", running: 1 })).toBe(
      "Its panes have a program that is still running. Closing the tab ends it.",
    );
    expect(closingText({ what: "window", running: 3 })).toBe(
      "Its panes have 3 programs that are still running. Closing the window ends them.",
    );
  });
});

describe("asking before a command the core says would end programs", () => {
  const notice = {
    type: "confirm_requested",
    title: "Restart the session host?",
    message: "Every program in every terminal ends.",
    confirm: "Restart",
    command: "lume.host.restart",
    args: { confirmed: true },
  } as const;

  function confirming(ok: boolean) {
    const prompts = new PromptStore();
    const asked: Confirmation[] = [];
    const ran: { id: string; args: unknown }[] = [];
    let refocused = 0;
    prompts.subscribe(() => {
      const current = prompts.current();
      if (current?.kind !== "confirm") return;
      asked.push(current.confirmation);
      current.answer(ok);
    });
    const app = {
      prompts,
      views: { focus: () => void (refocused += 1) },
      commands: { run: (id: string, args: unknown) => void ran.push({ id, args }) },
    } as unknown as ConfirmingApp;
    return { app, asked, ran, refocused: () => refocused };
  }

  it("asks with what the core said, and runs the command as given on a yes", async () => {
    const { app, asked, ran, refocused } = confirming(true);
    await confirmRequested(app, notice);
    expect(asked).toEqual([
      {
        title: "Restart the session host?",
        message: "Every program in every terminal ends.",
        confirm: "Restart",
      },
    ]);
    expect(ran).toEqual([{ id: "lume.host.restart", args: { confirmed: true } }]);
    expect(refocused()).toBe(0);
  });

  it("runs nothing on a no, and gives the keyboard back", async () => {
    const { app, ran, refocused } = confirming(false);
    await confirmRequested(app, notice);
    expect(ran).toEqual([]);
    expect(refocused()).toBe(1);
  });

  it("is one prompt at a time, answered once", async () => {
    const prompts = new PromptStore();
    const first = prompts.confirm({ title: "A?", message: "a", confirm: "Go" });
    const second = prompts.confirm({ title: "B?", message: "b", confirm: "Go" });
    const current = prompts.current();
    expect(current?.kind === "confirm" && current.confirmation.title).toBe("A?");
    current?.answer(true);
    current?.answer(false);
    expect(await first).toBe(true);
    const next = prompts.current();
    expect(next?.kind === "confirm" && next.confirmation.title).toBe("B?");
    next?.answer(false);
    expect(await second).toBe(false);
    expect(prompts.current()).toBeNull();
  });
});
