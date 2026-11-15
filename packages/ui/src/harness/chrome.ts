/**
 * The window's own page, run against a stand-in for the core, and held to what a page
 * must do however much its terminals print.
 *
 * Three panes are flooded with output and with changes of title while the renders of
 * the component tree are counted: there must be none, since a terminal's output is
 * drawn outside the tree. Then a tab is switched, which must render, so that a count of
 * nought cannot come from a counter that does not count. A tab that is off the screen
 * must hold no renderer and still take its sessions' output. And nothing the page does
 * may be refused by the content policy it is served under.
 */

import { createElement, Profiler } from "react";
// The renderer that reports its renders even in a production bundle.
import { createRoot } from "react-dom/profiling";

import { selectActiveTab } from "../layout/selectors.ts";
import { startShell, type ShellApp } from "../shell/boot.ts";
import { Shell } from "../shell/Shell.tsx";
import "../styles.css";
import { FakeCore, FIXTURE_TABS } from "./fake-core.ts";
import {
  CHROME_FLOOD_CHUNK_BYTES,
  CHROME_SETTLE_MS,
  CHROME_START_TIMEOUT_MS,
  COMMIT_TEST_MS,
} from "./tuning.ts";

/** What the suite reports. */
export interface ChromeReport {
  readonly ok: boolean;
  /** Renders of the component tree while the panes were flooded. Must be nought. */
  readonly outputCommits: number;
  /** How much output the panes were sent meanwhile. */
  readonly outputBytes: number;
  /** Renders when a tab was switched. Must be more than nought. */
  readonly switchCommits: number;
  /** Renderers alive with the first tab on show, with the other, and with the first again. */
  readonly renderers: {
    readonly firstTab: number;
    readonly otherTab: number;
    readonly back: number;
  };
  /** What the content policy refused. Must be nothing. */
  readonly policyViolations: readonly string[];
  readonly problems: readonly string[];
}

const PANES = ["p1", "p2", "p3"] as const;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

async function until(what: string, limitMs: number, done: () => boolean): Promise<void> {
  const deadline = performance.now() + limitMs;
  while (!done()) {
    if (performance.now() > deadline) throw new Error(`${what} never happened`);
    await sleep(20);
  }
}

function liveRenderers(app: ShellApp): number {
  return Object.values(app.stats().renderers).reduce((sum, count) => sum + count, 0);
}

/** Whether the screen of a pane's terminal shows `text`. */
function shows(app: ShellApp, pane: string, text: string): boolean {
  try {
    return (
      app.views
        .get(pane)
        ?.screen()
        .lines.some((line) => line.includes(text)) ?? false
    );
  } catch {
    // The view has no session yet.
    return false;
  }
}

/** A chunk of numbered lines, with a change of title in it. */
function chunk(pane: string, from: number): { bytes: Uint8Array; next: number } {
  let text = `\u001b]0;${pane} at ${from}\u0007`;
  let line = from;
  while (text.length < CHROME_FLOOD_CHUNK_BYTES) {
    text += `${pane} line ${line} of a flood that is here only to be drawn\r\n`;
    line += 1;
  }
  return { bytes: new TextEncoder().encode(text), next: line };
}

/** Runs the suite. `progress` is told what is being done. */
export async function runChrome(progress: (text: string) => void): Promise<ChromeReport> {
  const policyViolations: string[] = [];
  document.addEventListener("securitypolicyviolation", (event) => {
    policyViolations.push(`${event.violatedDirective}: ${event.blockedURI || event.sample}`);
  });
  const problems: string[] = [];

  progress("starting the window's page");
  const core = new FakeCore();
  const app = await startShell(core);
  const container = document.createElement("div");
  document.body.append(container);
  let commits = 0;
  createRoot(container).render(
    createElement(
      Profiler,
      { id: "shell", onRender: () => (commits += 1) },
      createElement(Shell, { app }),
    ),
  );
  await until("the page attaching to its panes' sessions", CHROME_START_TIMEOUT_MS, () =>
    core.attached(),
  );
  await until("the first tab's panes being drawn", CHROME_START_TIMEOUT_MS, () => {
    return document.querySelectorAll(".pane-card canvas").length === 2;
  });
  await sleep(CHROME_SETTLE_MS);
  const firstTab = liveRenderers(app);
  if (firstTab !== 2) problems.push(`two panes are on show and ${firstTab} renderers are alive`);

  progress("flooding three panes and counting renders");
  const lines = { p1: 1, p2: 1, p3: 1 };
  let outputBytes = 0;
  commits = 0;
  const flooded = performance.now() + COMMIT_TEST_MS;
  while (performance.now() < flooded) {
    for (const pane of PANES) {
      const { bytes, next } = chunk(pane, lines[pane]);
      lines[pane] = next;
      outputBytes += core.output(pane, bytes);
    }
    // A frame is given the chance to be drawn between rounds.
    await new Promise((resolve) => requestAnimationFrame(resolve));
  }
  await sleep(CHROME_SETTLE_MS);
  const outputCommits = commits;
  if (outputCommits !== 0) {
    problems.push(`the component tree rendered ${outputCommits} times for terminal output`);
  }
  for (const pane of PANES) {
    const last = `${pane} line ${lines[pane] - 1} of`;
    if (!shows(app, pane, last)) problems.push(`${pane} does not show the last line it was sent`);
  }
  if (app.titles.get("p1") === undefined) problems.push("the titles the panes set did not arrive");

  progress("switching tabs");
  commits = 0;
  core.activate(FIXTURE_TABS.second);
  await until("the other tab being shown", CHROME_START_TIMEOUT_MS, () => {
    return selectActiveTab(app.layout.getState()) === FIXTURE_TABS.second;
  });
  await sleep(CHROME_SETTLE_MS);
  const switchCommits = commits;
  if (switchCommits === 0) {
    problems.push("switching tabs rendered nothing, so the count of renders cannot be trusted");
  }
  const otherTab = liveRenderers(app);
  if (otherTab !== 1) problems.push(`one pane is on show and ${otherTab} renderers are alive`);
  if (document.querySelectorAll(".pane-card").length !== 1) {
    problems.push("the panes of the tab that is off the screen are still in the page");
  }

  // A pane that is off the screen still takes what its session prints.
  const unseen = new TextEncoder().encode("\r\nprinted while nobody was looking\r\n");
  core.output("p1", unseen);
  await sleep(CHROME_SETTLE_MS);
  if (!shows(app, "p1", "printed while nobody was looking")) {
    problems.push("a pane that is off the screen did not take its session's output");
  }

  core.activate(FIXTURE_TABS.first);
  await until("the first tab's panes being drawn again", CHROME_START_TIMEOUT_MS, () => {
    return document.querySelectorAll(".pane-card canvas").length === 2;
  });
  await sleep(CHROME_SETTLE_MS);
  const back = liveRenderers(app);
  if (back !== 2) problems.push(`two panes are on show again and ${back} renderers are alive`);

  for (const name of core.unanswered) {
    problems.push(`the page called ${name}, which the stand-in for the core does not answer`);
  }
  for (const violation of policyViolations) {
    problems.push(`the content policy refused ${violation}`);
  }
  return {
    ok: problems.length === 0,
    outputCommits,
    outputBytes,
    switchCommits,
    renderers: { firstTab, otherTab, back },
    policyViolations,
    problems,
  };
}
