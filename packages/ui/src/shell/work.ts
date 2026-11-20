/**
 * What each pane of the window is doing that the user may want to know about: a
 * program waiting on them, a command or an agent at work, something that finished while
 * they looked elsewhere.
 *
 * The Activity list in the top bar shows these, and the question asked before a close
 * names the pane it is about the same way. Everything is read from what the panes'
 * sessions reported, through the same description their headers and tabs use.
 */

import { selectWindowPanes } from "../layout/selectors.ts";
import type { PaneId, TabId } from "../layout/types.ts";
import type { ShellApp } from "./boot.ts";
import { describeNow, describeTabNow } from "./describing.ts";

/**
 * How a pane's work stands: it waits on the user, it is at work, or it has finished,
 * well or not, since the user last looked at it.
 */
export type WorkState = "wait" | "work" | "done" | "failed";

/** One pane's work, as the Activity list and the close question say it. */
export interface PaneWork {
  readonly pane: PaneId;
  readonly tab: TabId | null;
  /** What the tab the pane is in is called. */
  readonly tabLabel: string;
  /** What is doing the work: the program, or the command line's program. */
  readonly who: string;
  /** One line on what it is doing, or how it ended. */
  readonly line: string;
  readonly state: WorkState;
  /** Whether the program reports its own status, as coding agents do. */
  readonly agent: boolean;
  /** Whether the pane has something the user has not looked at. */
  readonly unread: boolean;
}

/** The order the Activity list keeps: what waits on the user first. */
const ORDER: Readonly<Record<WorkState, number>> = { wait: 0, failed: 1, done: 2, work: 3 };

/** A pane's work, or nothing when it is idle and has nothing to show. */
export function workOf(app: ShellApp, pane: PaneId): PaneWork | null {
  const description = describeNow(app, pane);
  const report = app.reports.get(pane);
  const state = app.layout.getState();
  const tab = state.ready ? (state.paneTabs[pane] ?? null) : null;
  const tabLabel = tab === null ? description.name : describeTabNow(app, tab).label;
  const unread = description.activity === "attention" || description.activity === "attention-error";
  const agent = report.statuses.length > 0;
  const base = { pane, tab, tabLabel, who: description.name, agent, unread };

  if (description.ended) {
    if (!unread) return null;
    const failed = description.activity === "attention-error";
    return { ...base, state: failed ? "failed" : "done", line: description.ended.reason };
  }
  const status = description.status;
  if (status?.state === "blocked") {
    return { ...base, state: "wait", line: status.tooltip ?? "Waiting on you" };
  }
  if (status?.state === "working" || description.running) {
    const command =
      report.command?.state === "running" && report.command.command !== undefined
        ? report.command.command
        : undefined;
    const line = status?.tooltip ?? (command === undefined ? "Running" : `Running ${command}`);
    return { ...base, state: "work", line };
  }
  // A pane that only printed something is not news enough for the list.
  if (description.activity === "idle" || description.activity === "output") return null;
  if (status?.state === "error" || description.failed !== null) {
    const line =
      description.failed !== null
        ? `Finished with exit code ${description.failed}`
        : (status?.tooltip ?? "Failed");
    return { ...base, state: "failed", line };
  }
  if (status?.state === "done") {
    return { ...base, state: "done", line: status.tooltip ?? "Finished" };
  }
  if (report.command?.state === "finished") {
    return { ...base, state: "done", line: "Finished with exit code 0" };
  }
  return { ...base, state: "done", line: "Wants your attention" };
}

/** The work of every pane of the window, what waits on the user first. */
export function windowWork(app: ShellApp): PaneWork[] {
  const state = app.layout.getState();
  if (!state.ready) return [];
  const work = selectWindowPanes(state)
    .map((pane) => workOf(app, pane))
    .filter((item): item is PaneWork => item !== null);
  return work.sort((a, b) => ORDER[a.state] - ORDER[b.state]);
}

/** How the close question and the Activity list say what a pane's work is doing. */
export function workRest(work: PaneWork): string {
  switch (work.state) {
    case "wait":
      return "is waiting for your answer";
    case "work":
      return work.agent ? "is still working" : "is still running";
    case "failed":
      return "failed";
    case "done":
      return "has finished";
  }
}
