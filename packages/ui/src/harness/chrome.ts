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
 *
 * Then the page is put through a number of scenes: one pane and several, tabs along the
 * top and down the side, a pane that floats, a panel at a side, every kind of status
 * item, the palette, a pane being searched. Each scene is written down as what it is
 * made of, in the dark look and in the light, and compared with what it was made of
 * when the scene was last looked at and found right; and each is checked for what a
 * keyboard and a screen reader need of it. The settings window, which is a page of its
 * own, is put through the same.
 */

import { CSS_VARIABLES, type TokenName } from "@lume/tokens";
import { createElement, Profiler } from "react";
// The renderer that reports its renders even in a production bundle.
import { createRoot } from "react-dom/profiling";

import { selectActiveTab } from "../layout/selectors.ts";
import { startSettings } from "../settings/boot.ts";
import { SettingsWindow } from "../settings/SettingsWindow.tsx";
import { startShell, type ShellApp } from "../shell/boot.ts";
import { Shell } from "../shell/Shell.tsx";
import { STATUS_FIXTURES } from "../status/fixtures.ts";
import "../styles.css";
import { audit, type AuditNode } from "./audit.ts";
import { FakeCore, FIXTURE_TABS, type FakeCoreOptions } from "./fake-core.ts";
import { FIXTURE_BINDINGS, FIXTURE_COMMANDS } from "./fixture-commands.ts";
import { readReference, saveFile, type HarnessInfo } from "./host.ts";
import { CONNECT_SCENES } from "./connect-scenes.ts";
import { PANE_SCENES } from "./pane-scenes.ts";
import { TAB_SCENES } from "./tab-scenes.ts";
import type { Scene } from "./scene.ts";
import { describePage, differences, plainColor, recordText, type Part } from "./structure.ts";
import {
  CHROME_FLOOD_CHUNK_BYTES,
  CHROME_SETTLE_MS,
  CHROME_START_TIMEOUT_MS,
  COMMIT_TEST_MS,
  SCENE_BOX_TOLERANCE_PX,
  SCENE_HEIGHT_PX,
  SCENE_WIDTH_PX,
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
  /** The scenes the page was put through, and how each came out. */
  readonly scenes: readonly SceneResult[];
  /** What the content policy refused. Must be nothing. */
  readonly policyViolations: readonly string[];
  readonly problems: readonly string[];
}

/** How one scene came out. */
export interface SceneResult {
  readonly name: string;
  /** `same`, `different`, `no-reference`, or `written` in a run that replaces references. */
  readonly verdict: "same" | "different" | "no-reference" | "written" | "not-compared";
  /** How many things the check for keyboards and screen readers found wrong. */
  readonly auditProblems: number;
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

// The scenes.

/** What the pane that is searched in a scene has printed, and what is looked for in it. */
const SEARCHED_OUTPUT = "one needle\r\ntwo\r\nanother needle and a NEEDLE\r\n";
const SEARCHED_FOR = "needle";

/** The scenes of the window's page, in the order they are run. */
const SCENES: readonly Scene[] = [
  { name: "one-pane", window: { tabs: [1] } },
  { name: "two-panes", window: { tabs: [2] } },
  { name: "three-panes", window: { tabs: [3] } },
  { name: "four-panes", window: { tabs: [4] } },
  // The page draws the window's own buttons here, as it does without the system's bar.
  { name: "tabs-top", window: { tabs: [2, 1], titleBar: "custom" } },
  { name: "tabs-left", window: { tabs: [2, 1], tabPlacement: "left" } },
  { name: "floating-pane", window: { tabs: [2], floating: true, minimised: true } },
  { name: "dock-panel", window: { tabs: [2], dockPanel: "right" } },
  {
    name: "status-items",
    window: { tabs: [1] },
    prepare: (app) => {
      for (const item of STATUS_FIXTURES) app.status.getState().register(item);
    },
  },
  {
    name: "palette",
    window: { tabs: [2], commands: FIXTURE_COMMANDS, bindings: FIXTURE_BINDINGS },
    prepare: (app) => app.palette.getState().open(),
  },
  {
    name: "search",
    window: { tabs: [2], commands: FIXTURE_COMMANDS, bindings: FIXTURE_BINDINGS },
    prepare: (app, core) => {
      const pane = core.panesOf(core.tabs[0] ?? "")[0] ?? "";
      core.output(pane, new TextEncoder().encode(SEARCHED_OUTPUT));
      app.commands.runInPage("lume.search.open", { pane, query: SEARCHED_FOR });
    },
    check: (page) => {
      const said = page.querySelector(".search-count")?.textContent ?? "";
      return said === "1 of 3" ? [] : [`the search bar says ${JSON.stringify(said)}, not 1 of 3`];
    },
  },
  ...PANE_SCENES,
  ...CONNECT_SCENES,
  ...TAB_SCENES,
];

/** One state of the settings window to write down. */
interface SettingsScene {
  readonly name: string;
  /** The page the window is opened at. */
  readonly page: string;
  /** What is typed into the field that searches the settings, if anything. */
  readonly query?: string;
}

const SETTINGS_SCENES: readonly SettingsScene[] = [
  { name: "settings", page: "appearance" },
  { name: "settings-terminal", page: "terminal" },
  { name: "settings-search", page: "general", query: "cursor" },
];

/** The tokens whose values are written down with a scene: the look's main numbers. */
const LOOK_TOKENS: readonly TokenName[] = [
  "color.surface",
  "color.card",
  "color.card-header",
  "color.border",
  "color.border-focus",
  "color.text",
  "color.text-muted",
  "color.accent",
  "terminal.bg",
  "terminal.fg",
  "space.gap",
  "radius.card",
  "size.header",
  "size.topbar",
];

/** A scene as it is kept in its reference file. */
interface SceneRecord {
  /** What the page is made of in the dark look. */
  readonly structure: Part;
  /** The same in the light look, when it is not the same. */
  readonly light?: Part;
  /** The look's numbers as the page has them, and a few of the colours it draws with. */
  readonly colors: {
    readonly dark: Readonly<Record<string, string>>;
    readonly light: Readonly<Record<string, string>>;
  };
}

/** The look as the page has it: the tokens it was given, and what a card is drawn with. */
function lookOf(shell: Element): Record<string, string> {
  const look: Record<string, string> = {};
  const root = getComputedStyle(document.documentElement);
  for (const token of LOOK_TOKENS) {
    const value = root.getPropertyValue(CSS_VARIABLES[token]).trim();
    look[token] = value.startsWith("#") ? plainColor(value) : value;
  }
  // That a token was handed over does not show it is used: a card is asked as well.
  const card = shell.querySelector(".pane-card");
  if (card) {
    const drawn = getComputedStyle(card);
    look["a pane's background"] = plainColor(drawn.backgroundColor);
    look["a pane's corner"] = drawn.borderTopLeftRadius;
  }
  look["the window's text"] = plainColor(getComputedStyle(shell).color);
  return look;
}

/** A window's page, started on a stand-in core and drawn in a box of a known size. */
export interface Mounted {
  readonly core: FakeCore;
  readonly app: ShellApp;
  /** The box the window fills. */
  readonly container: HTMLElement;
  /** Puts the page away: its components, its views and its renderers. */
  unmount(): void;
}

/**
 * Starts a window's page as `asked` describes it and waits until it has attached to
 * its sessions and drawn the panes it shows. `limitMs` is how long that may take.
 */
export async function mountWindow(
  asked: FakeCoreOptions,
  limitMs: number = CHROME_START_TIMEOUT_MS,
): Promise<Mounted> {
  const core = new FakeCore(asked);
  const app = await startShell(core);
  // A box of its own size for the window to fill, whatever size the real window is.
  const container = document.createElement("div");
  container.style.setProperty("position", "fixed");
  container.style.setProperty("left", "0");
  container.style.setProperty("top", "0");
  container.style.setProperty("width", `${SCENE_WIDTH_PX}px`);
  container.style.setProperty("height", `${SCENE_HEIGHT_PX}px`);
  // What is laid out against the window is laid out against this box.
  container.style.setProperty("contain", "strict");
  document.body.append(container);
  const root = createRoot(container);
  const mounted: Mounted = {
    core,
    app,
    container,
    unmount: () => {
      root.unmount();
      app.dispose();
      container.remove();
    },
  };
  try {
    root.render(createElement(Shell, { app }));
    await until("the page attaching to its sessions", limitMs, () => core.attached());
    const first = core.tabs[0];
    const drawn = (first ? core.panesOf(first).length : 0) + (asked.floating === true ? 1 : 0);
    await until("its panes being drawn", limitMs, () => {
      return container.querySelectorAll(".pane-card canvas").length === drawn;
    });
  } catch (error) {
    mounted.unmount();
    throw error;
  }
  return mounted;
}

/**
 * Shows one scene of the window's page and leaves it on show, in the look asked for, for
 * a person to look at. Nothing is written down or compared.
 */
export async function showScene(name: string, scheme: "light" | "dark"): Promise<string[]> {
  const settings = SETTINGS_SCENES.find((candidate) => candidate.name === name);
  if (settings) {
    // The settings window is written down in the dark look only, as it is shown here.
    const mounted = await mountSettings(settings.page);
    if (settings.query !== undefined) {
      const field = mounted.container.querySelector<HTMLInputElement>(".settings-search input");
      if (field) typeInto(field, settings.query);
    }
    await sleep(CHROME_SETTLE_MS);
    return [];
  }
  const scene = SCENES.find((candidate) => candidate.name === name);
  if (!scene) throw new Error(`there is no scene called ${name}`);
  const mounted = await mountWindow({ ...scene.window, scheme });
  await scene.prepare?.(mounted.app, mounted.core);
  await sleep(CHROME_SETTLE_MS);
  return scene.check?.(mounted.container) ?? [];
}

/**
 * Starts the settings window's page at `page` and draws it in a box of a known size.
 * The page has nothing to put away but its components: it shows no terminals.
 */
async function mountSettings(
  page: string,
): Promise<Pick<Mounted, "core" | "container" | "unmount">> {
  const core = new FakeCore({ titleBar: "custom" });
  const app = await startSettings(core, `#${page}`);
  const container = document.createElement("div");
  container.style.setProperty("position", "fixed");
  container.style.setProperty("left", "0");
  container.style.setProperty("top", "0");
  container.style.setProperty("width", `${SCENE_WIDTH_PX}px`);
  container.style.setProperty("height", `${SCENE_HEIGHT_PX}px`);
  container.style.setProperty("contain", "strict");
  document.body.append(container);
  const root = createRoot(container);
  root.render(createElement(SettingsWindow, { app }));
  const unmount = (): void => {
    root.unmount();
    container.remove();
  };
  try {
    await until("the settings being listed", CHROME_START_TIMEOUT_MS, () => {
      return container.querySelector(".settings-row") !== null;
    });
  } catch (error) {
    unmount();
    throw error;
  }
  return { core, container, unmount };
}

/** Types into a field the way a user would, so that the component hears of it. */
function typeInto(field: HTMLInputElement, text: string): void {
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  set?.call(field, text);
  field.dispatchEvent(new Event("input", { bubbles: true }));
}

async function runSettingsScene(
  scene: SettingsScene,
  info: HarnessInfo | null,
  problems: string[],
): Promise<SceneResult> {
  const mounted = await mountSettings(scene.page);
  try {
    if (scene.query !== undefined) {
      const field = mounted.container.querySelector<HTMLInputElement>(".settings-search input");
      if (!field) throw new Error("the settings window has no field to search with");
      typeInto(field, scene.query);
    }
    return await writeScene(scene.name, mounted, ".settings-window", () => [], info, problems);
  } finally {
    mounted.unmount();
  }
}

async function runScene(
  scene: Scene,
  info: HarnessInfo | null,
  problems: string[],
): Promise<SceneResult> {
  const mounted = await mountWindow(scene.window);
  try {
    await scene.prepare?.(mounted.app, mounted.core);
    const also = (): string[] => scene.check?.(mounted.container) ?? [];
    return await writeScene(scene.name, mounted, ".shell", also, info, problems);
  } finally {
    mounted.unmount();
  }
}

/**
 * Writes a page down once it has settled, in both looks, checks it for what a keyboard
 * and a screen reader need, and compares it with its reference. `also` is asked for
 * anything else that is wrong with it.
 */
async function writeScene(
  name: string,
  mounted: Pick<Mounted, "core" | "container">,
  root: string,
  also: () => string[],
  info: HarnessInfo | null,
  problems: string[],
): Promise<SceneResult> {
  const { core, container } = mounted;
  await sleep(CHROME_SETTLE_MS);
  for (const problem of also()) problems.push(`${name}: ${problem}`);

  const page = container.querySelector(root);
  if (!page) throw new Error("the page drew no window");
  const write = (): Part => {
    const origin = container.getBoundingClientRect();
    return describePage(
      page as unknown as AuditNode,
      (node) => (node as unknown as Element).getBoundingClientRect(),
      origin,
    );
  };

  const structure = write();
  const dark = lookOf(page);
  const found = audit(page as unknown as AuditNode);
  for (const problem of found) problems.push(`${name}: ${problem}`);

  core.setScheme("light");
  await until("the light look", CHROME_START_TIMEOUT_MS, () => {
    return document.documentElement.dataset["scheme"] === "light";
  });
  await sleep(CHROME_SETTLE_MS);
  const inLight = write();
  const light = lookOf(page);
  const sameInLight = differences(structure, inLight, 0).length === 0;
  const record: SceneRecord = {
    structure,
    ...(sameInLight ? {} : { light: inLight }),
    colors: { dark, light },
  };

  for (const call of core.unanswered) {
    problems.push(`${name}: the page called ${call}, which the stand-in does not answer`);
  }
  const verdict = await compareScene(name, record, info, problems);
  return { name, verdict, auditProblems: found.length };
}

/** Compares a scene with its reference file, or writes the file in a run that does. */
async function compareScene(
  name: string,
  record: SceneRecord,
  info: HarnessInfo | null,
  problems: string[],
): Promise<SceneResult["verdict"]> {
  // Opened in a browser there is no core to read a file from or write one through.
  if (!info) return "not-compared";
  const file = `chrome/${name}.json`;
  const text = new TextEncoder().encode(recordText(record));
  if (info.update) {
    await saveFile("references", file, text);
    return "written";
  }
  const kept = await readReference(file);
  if (!kept) {
    await saveFile("out", file, text).catch(() => undefined);
    problems.push(`${name}: there is no reference to compare the scene with`);
    return "no-reference";
  }
  const reference = JSON.parse(new TextDecoder().decode(kept)) as SceneRecord;
  const differing = [
    ...differences(reference.structure, record.structure, SCENE_BOX_TOLERANCE_PX),
    ...differences(
      reference.light ?? reference.structure,
      record.light ?? record.structure,
      SCENE_BOX_TOLERANCE_PX,
    ).map((difference) => `in the light look, ${difference}`),
  ];
  for (const look of ["dark", "light"] as const) {
    const before = reference.colors[look];
    const now = record.colors[look];
    for (const key of new Set([...Object.keys(before), ...Object.keys(now)])) {
      if (before[key] !== now[key]) {
        differing.push(
          `in the ${look} look, ${key} was ${before[key] ?? "not there"} and is ${now[key] ?? "not there"}`,
        );
      }
    }
  }
  if (differing.length === 0) return "same";
  await saveFile("out", file, text).catch(() => undefined);
  for (const difference of differing) problems.push(`${name}: ${difference}`);
  return "different";
}

/**
 * Runs the suite. `info` is what the core said of the run, if there is a core; `progress`
 * is told what is being done.
 */
export async function runChrome(
  info: HarnessInfo | null,
  progress: (text: string) => void,
): Promise<ChromeReport> {
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
  const root = createRoot(container);
  root.render(
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
      const from = lines[pane];
      const { bytes, next } = chunk(pane, from);
      lines[pane] = next;
      outputBytes += core.output(pane, bytes);
      // What the sessions say of themselves changes as often: the title, where the
      // shell is, and that a pane printed while nobody looked. None of it may render.
      core.event(pane, { type: "title", title: `${pane} at ${from}` });
      core.event(pane, { type: "cwd", url: `file://FIXTURE/C:/flood/${pane}/${from}` });
      core.event(pane, { type: "activity" });
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

  // The page that was flooded is put away before the scenes are set up one by one.
  root.unmount();
  app.dispose();
  container.remove();

  const scenes: SceneResult[] = [];
  for (const scene of SCENES) {
    progress(`the scene ${scene.name}`);
    try {
      scenes.push(await runScene(scene, info, problems));
    } catch (error) {
      problems.push(`${scene.name}: ${error instanceof Error ? error.message : String(error)}`);
      scenes.push({ name: scene.name, verdict: "not-compared", auditProblems: 0 });
    }
  }
  for (const scene of SETTINGS_SCENES) {
    progress(`the scene ${scene.name}`);
    try {
      scenes.push(await runSettingsScene(scene, info, problems));
    } catch (error) {
      problems.push(`${scene.name}: ${error instanceof Error ? error.message : String(error)}`);
      scenes.push({ name: scene.name, verdict: "not-compared", auditProblems: 0 });
    }
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
    scenes,
    policyViolations,
    problems,
  };
}
