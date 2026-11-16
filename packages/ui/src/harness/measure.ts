/**
 * How the window's own page holds up under load, in numbers.
 *
 * The benchmarks of the terminal time one terminal at a time against a real session.
 * These time the page as a whole, on a stand-in core that can print as fast as asked:
 * many panes at once on each renderer tier, with what they cost in memory; a divider
 * dragged while its panes print; how long a tab takes to be drawn again after its
 * panes gave up their renderers; and how long a search of a long scrollback takes, and
 * the longest it keeps the page from anything else. The numbers are held to the
 * benchmarks' budgets.
 */

import type { Tier } from "@lume/renderer";

import type { PaneId } from "../layout/types.ts";
import { mountWindow, type Mounted } from "./chrome.ts";
import { FIXTURE_COMMANDS } from "./fixture-commands.ts";
import { nextFrame, sleep } from "./timing.ts";
import {
  CHROME_FLOOD_CHUNK_BYTES,
  CHROME_SETTLE_MS,
  DRAG_MOVED_MIN_PX,
  DRAG_MS,
  DRAG_PERIOD_MS,
  DRAG_REACH_PX,
  MANY_PANES_FLOOD_MS,
  MANY_PANES_PER_TAB,
  MANY_PANES_SCROLLBACK_LINES,
  MANY_PANES_TABS,
  SEARCH_LINES,
  SEARCH_LINES_PER_ROUND,
  SEARCH_MATCH_EVERY,
  SEARCH_TIMEOUT_MS,
  TAB_SWITCH_REST_MS,
  TAB_SWITCH_TIMEOUT_MS,
  TAB_SWITCHES,
  TIER_START_TIMEOUT_MS,
} from "./tuning.ts";

const MIB = 1024 * 1024;

/** The tiers a terminal can be drawn with, best first. */
const TIERS: readonly Tier[] = ["webgpu", "webgl2", "canvas2d"];

/** The pointer a mouse is: the one pointer a page may take hold of without a press. */
const MOUSE_POINTER = 1;

/** Many panes printing at once on one tier. */
export interface ManyPanes {
  /** How many panes there were, and how many of them were on show. */
  readonly panes: number;
  readonly shown: number;
  /** Frames the page was given each second while every pane printed. */
  readonly framesPerSecond: number;
  /** What was printed into the panes between them. */
  readonly outputMiB: number;
  /** The memory the terminals of the window hold between them, scrollback and all. */
  readonly wasmMiB: number;
  /** What the glyphs of the panes on show take. */
  readonly atlasMiB: number;
  /** The WebGL2 contexts alive. */
  readonly contexts: number;
  /** The renderers alive, by tier. */
  readonly renderers: Readonly<Record<Tier, number>>;
}

/** What the suite reports. */
export interface MeasureReport {
  readonly ok: boolean;
  /** Sixteen panes in four tabs, printing in all, for each tier the webview offers. */
  readonly panes16: Readonly<Partial<Record<Tier, ManyPanes>>>;
  /** A divider dragged to and fro while the panes at its sides print. */
  readonly drag: {
    readonly framesPerSecond: number;
    /** How far the divider went, at its furthest. */
    readonly reachedPx: number;
  };
  /** From a tab being asked for to its panes being drawn again. */
  readonly tabSwitch: {
    /** The middle one of the switches timed, on the WebGL2 tier. */
    readonly webgl2Ms?: number;
    readonly webgl2MaxMs?: number;
    readonly samples?: number;
  };
  /** A scrollback of many lines searched for a text that is on some of them. */
  readonly search: {
    /** How many lines the terminal held, and how many matches were found in them. */
    readonly lines?: number;
    readonly matches?: number;
    /** From the text being asked for to the last line having been looked at. */
    readonly completeMs?: number;
    /** The longest the search kept the page from doing anything else. */
    readonly maxSliceMs?: number;
  };
  readonly problems: readonly string[];
}

function round(value: number, places: number): number {
  const scale = 10 ** places;
  return Math.round(value * scale) / scale;
}

/** A chunk of numbered lines for a pane to print. */
function chunk(pane: PaneId, from: number): { bytes: Uint8Array; next: number } {
  let text = "";
  let line = from;
  while (text.length < CHROME_FLOOD_CHUNK_BYTES) {
    text += `${pane} line ${line} of a flood that is here only to be drawn\r\n`;
    line += 1;
  }
  return { bytes: new TextEncoder().encode(text), next: line };
}

/**
 * Prints into `panes` once a frame for `ms`, doing `each` between frames, and says how
 * many frames a second the page was given and how much was printed.
 */
async function flood(
  mounted: Mounted,
  panes: readonly PaneId[],
  ms: number,
  each: (elapsedMs: number) => void = () => undefined,
): Promise<{ framesPerSecond: number; bytes: number }> {
  const lines = new Map<PaneId, number>(panes.map((pane) => [pane, 1]));
  let bytes = 0;
  const frames: number[] = [];
  const started = performance.now();
  for (;;) {
    const now = performance.now();
    if (now - started >= ms) break;
    for (const pane of panes) {
      const piece = chunk(pane, lines.get(pane) ?? 1);
      lines.set(pane, piece.next);
      bytes += mounted.core.output(pane, piece.bytes);
    }
    each(now - started);
    await nextFrame();
    frames.push(performance.now());
  }
  const span = (frames[frames.length - 1] ?? 0) - (frames[0] ?? 0);
  return {
    framesPerSecond: span > 0 ? round(((frames.length - 1) / span) * 1000, 1) : 0,
    bytes,
  };
}

/** Sixteen panes in four tabs on one tier, or nothing if the webview lacks the tier. */
async function manyPanes(tier: Tier, problems: string[]): Promise<ManyPanes | undefined> {
  let mounted: Mounted;
  try {
    mounted = await mountWindow(
      {
        tabs: Array.from({ length: MANY_PANES_TABS }, () => MANY_PANES_PER_TAB),
        tier,
        scrollbackLines: MANY_PANES_SCROLLBACK_LINES,
      },
      TIER_START_TIMEOUT_MS,
    );
  } catch {
    // The webview does not offer the tier: there is nothing to time, and that is not
    // a failure of the page.
    return undefined;
  }
  try {
    const drawnWith = mounted.app.stats().renderers;
    if (drawnWith[tier] === 0) return undefined;
    const panes = mounted.core.panes;
    const { framesPerSecond, bytes } = await flood(mounted, panes, MANY_PANES_FLOOD_MS);
    await sleep(CHROME_SETTLE_MS);
    const stats = mounted.app.stats();
    const shown = Object.values(stats.renderers).reduce((sum, count) => sum + count, 0);
    if (shown !== MANY_PANES_PER_TAB) {
      problems.push(`${tier}: ${MANY_PANES_PER_TAB} panes are on show and ${shown} are drawn`);
    }
    return {
      panes: panes.length,
      shown,
      framesPerSecond,
      outputMiB: round(bytes / MIB, 1),
      wasmMiB: round(stats.wasmBytes / MIB, 1),
      atlasMiB: round(stats.atlasBytes / MIB, 2),
      contexts: stats.webgl2Contexts,
      renderers: stats.renderers,
    };
  } finally {
    mounted.unmount();
  }
}

/** A pointer event of the mouse's at a place, as a drag is made of. */
function pointer(type: string, x: number, y: number): PointerEvent {
  return new PointerEvent(type, {
    pointerId: MOUSE_POINTER,
    pointerType: "mouse",
    isPrimary: true,
    button: 0,
    buttons: type === "pointerup" ? 0 : 1,
    clientX: x,
    clientY: y,
    bubbles: true,
    cancelable: true,
  });
}

/** A divider dragged to and fro between two panes that print the whole time. */
async function dragDivider(problems: string[]): Promise<MeasureReport["drag"]> {
  const mounted = await mountWindow({ tabs: [2] });
  try {
    const divider = mounted.container.querySelector<HTMLElement>(".divider");
    if (!divider) {
      problems.push("the window of two panes has no divider to drag");
      return { framesPerSecond: 0, reachedPx: 0 };
    }
    const at = divider.getBoundingClientRect();
    const x = at.x + at.width / 2;
    const y = at.y + at.height / 2;
    divider.dispatchEvent(pointer("pointerdown", x, y));
    let reachedPx = 0;
    const { framesPerSecond } = await flood(mounted, mounted.core.panes, DRAG_MS, (elapsed) => {
      const swing = Math.sin((elapsed / DRAG_PERIOD_MS) * 2 * Math.PI) * DRAG_REACH_PX;
      divider.dispatchEvent(pointer("pointermove", x + swing, y));
      reachedPx = Math.max(reachedPx, Math.abs(divider.getBoundingClientRect().x - at.x));
    });
    divider.dispatchEvent(pointer("pointerup", x, y));
    await sleep(CHROME_SETTLE_MS);
    // A drag that did not take hold would time panes that only print.
    if (reachedPx < DRAG_MOVED_MIN_PX) {
      problems.push(`the divider was dragged and went ${Math.round(reachedPx)} pixels`);
    }
    return { framesPerSecond, reachedPx: Math.round(reachedPx) };
  } finally {
    mounted.unmount();
  }
}

/** Whether every pane of a tab is in the page with its terminal drawn. */
function drawn(mounted: Mounted, panes: readonly PaneId[]): boolean {
  return panes.every((pane) => {
    const view = mounted.app.views.get(pane);
    const canvas = mounted.container.querySelector(`.pane-card[data-pane="${pane}"] canvas`);
    return canvas !== null && view !== undefined && view.mounted && !view.needsFrame();
  });
}

/** Tabs of four panes switched between on the WebGL2 tier, each switch timed. */
async function switchTabs(problems: string[]): Promise<MeasureReport["tabSwitch"]> {
  let mounted: Mounted;
  try {
    mounted = await mountWindow(
      { tabs: [MANY_PANES_PER_TAB, MANY_PANES_PER_TAB], tier: "webgl2" },
      TIER_START_TIMEOUT_MS,
    );
  } catch {
    return {};
  }
  try {
    if (mounted.app.stats().renderers.webgl2 === 0) return {};
    const { core } = mounted;
    // Something on every screen, so a tab that is shown again has glyphs to draw.
    await flood(mounted, core.panes, CHROME_SETTLE_MS);
    const times: number[] = [];
    for (let turn = 0; turn < TAB_SWITCHES; turn++) {
      const tab = core.tabs[(turn + 1) % core.tabs.length];
      if (tab === undefined) break;
      const panes = core.panesOf(tab);
      await sleep(TAB_SWITCH_REST_MS);
      const asked = performance.now();
      core.activate(tab);
      while (!drawn(mounted, panes)) {
        if (performance.now() - asked > TAB_SWITCH_TIMEOUT_MS) {
          problems.push(`the tab ${tab} was not drawn again after it was shown`);
          return {};
        }
        // Looked at again as soon as the page has had a turn.
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
      times.push(performance.now() - asked);
    }
    const sorted = [...times].sort((a, b) => a - b);
    const middle = sorted[Math.floor(sorted.length / 2)];
    if (middle === undefined) return {};
    return {
      webgl2Ms: round(middle, 1),
      webgl2MaxMs: round(sorted[sorted.length - 1] ?? middle, 1),
      samples: sorted.length,
    };
  } finally {
    mounted.unmount();
  }
}

/**
 * Fills one pane's scrollback and searches it for a text that is on one line in every
 * so many, the way the user would: through the command that opens the search bar.
 */
async function searchScrollback(problems: string[]): Promise<MeasureReport["search"]> {
  const mounted = await mountWindow({
    tabs: [1],
    scrollbackLines: SEARCH_LINES,
    commands: FIXTURE_COMMANDS,
  });
  try {
    const { app, core } = mounted;
    const pane = core.panesOf(core.tabs[0] ?? "")[0];
    const view = pane === undefined ? undefined : app.views.get(pane);
    if (pane === undefined || !view) {
      problems.push("there was no pane to search");
      return {};
    }
    const encoder = new TextEncoder();
    for (let line = 0; line < SEARCH_LINES;) {
      let text = "";
      for (let count = 0; count < SEARCH_LINES_PER_ROUND && line < SEARCH_LINES; count++, line++) {
        const marked = line % SEARCH_MATCH_EVERY === 0 ? " with the needle in it" : "";
        text += `line ${line} of a long scrollback${marked}\r\n`;
      }
      core.output(pane, encoder.encode(text));
      // The page is given a turn, as it is between a session's frames.
      await nextFrame();
    }
    // The search is timed over the whole scrollback, so it waits until the terminal
    // has taken the last line: what a page is sent is not all in its terminal at once.
    const last = `line ${SEARCH_LINES - 1} of`;
    const deadline = performance.now() + SEARCH_TIMEOUT_MS;
    while (!view.screen().lines.some((line) => line.includes(last))) {
      if (performance.now() > deadline) {
        problems.push("the terminal did not take the lines it was to be searched for");
        return {};
      }
      await sleep(10);
    }
    await sleep(CHROME_SETTLE_MS);

    app.commands.runInPage("lume.search.open", { pane, query: "the needle" });
    for (;;) {
      const run = app.search.lastRun(pane);
      if (run?.completeMs !== undefined && !app.search.of(pane).running) break;
      if (performance.now() > deadline) {
        problems.push("the search of a long scrollback did not end in time");
        return {};
      }
      await sleep(10);
    }
    const run = app.search.lastRun(pane);
    const matches = app.search.of(pane).total;
    const expected = Math.ceil(SEARCH_LINES / SEARCH_MATCH_EVERY);
    // The oldest lines scroll out of what is kept once the newest are printed.
    if (matches < expected * 0.9 || matches > expected) {
      problems.push(`the search found ${matches} matches where about ${expected} were printed`);
    }
    return {
      lines: SEARCH_LINES,
      matches,
      completeMs: round(run?.completeMs ?? 0, 1),
      maxSliceMs: round(run?.maxSliceMs ?? 0, 2),
    };
  } finally {
    mounted.unmount();
  }
}

/** Runs the measurements. `progress` is told what is being done. */
export async function runMeasure(progress: (text: string) => void): Promise<MeasureReport> {
  const problems: string[] = [];
  const panes16: Partial<Record<Tier, ManyPanes>> = {};
  for (const tier of TIERS) {
    progress(`sixteen panes printing, drawn with ${tier}`);
    const measured = await manyPanes(tier, problems);
    if (measured) panes16[tier] = measured;
  }
  if (Object.keys(panes16).length === 0) {
    problems.push("the webview drew the panes with no tier at all");
  }

  progress("dragging a divider between two panes that print");
  const drag = await dragDivider(problems);

  progress("switching between tabs of four panes");
  const tabSwitch = await switchTabs(problems);

  progress("searching a long scrollback");
  const search = await searchScrollback(problems);

  return { ok: problems.length === 0, panes16, drag, tabSwitch, search, problems };
}
