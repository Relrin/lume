/**
 * The scenes of what a pane's header, its tab and its marker show of the program in it:
 * a shell at its prompt after a command failed, a command running in a distribution, a
 * program on another machine waiting on the user, a connection that was lost; headers
 * too narrow for all of it; tabs that gathered something while nobody looked; and a
 * pane drawn in the look its profile asks for.
 */

import { LIGHT_TOKENS } from "@lume/tokens";

import type { ProfileList } from "../core/types.ts";
import type { ShellApp } from "../shell/boot/index.ts";
import type { Launcher, SessionEvent } from "../transport/types.ts";
import type { FakeCore, FixturePane } from "./fake-core.ts";
import { FIXTURE_COMMANDS } from "./fixture-commands.ts";
import type { Scene } from "./scene.ts";
import { CHROME_START_TIMEOUT_MS } from "./tuning.ts";

/** How often a scene looks again for what it waits for. */
const POLL_MS = 20;

/** The whole a split's shares add up to. */
const WHOLE = 1_000_000;

/** The left pane's share when its header is to be narrower than every breakpoint but one. */
const NARROW_SHARE = 230_000;

/** The left pane's share when its header is to be the narrowest there is. */
const TINY_SHARE = 95_000;

const PWSH: Launcher = {
  type: "shell",
  program: "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
  args: [],
  login: false,
};
const UBUNTU: Launcher = { type: "wsl", distro: "Ubuntu" };
const STAGING: Launcher = { type: "ssh", host: "dev-staging" };
const BOX: Launcher = { type: "ssh", host: "box", user: "mira" };

/** What the four panes of the header scenes run, as the core says when the page attaches. */
const FOUR: Readonly<Record<string, FixturePane>> = {
  p1: { launcher: PWSH, profile: "auto.pwsh" },
  p2: { launcher: UBUNTU, profile: "auto.wsl.Ubuntu" },
  p3: { launcher: STAGING, profile: "auto.ssh.dev-staging" },
  p4: { launcher: BOX },
};

/** What the sessions of the header scenes report, by pane. */
const REPORTS: Readonly<Record<string, readonly SessionEvent[]>> = {
  p1: [
    { type: "cwd", url: "file://FIXTURE/C:/Users/fixture/src/lume" },
    { type: "foreground", pid: 10, name: "pwsh" },
    { type: "command_started", command: "cargo test" },
    { type: "command_finished", exit: 3 },
  ],
  p2: [
    { type: "cwd", url: "file://FIXTURE/home/fixture/notes" },
    { type: "foreground", pid: 20, name: "wsl" },
    { type: "command_started", command: "tar czf notes.tgz ." },
    { type: "activity" },
  ],
  p3: [
    { type: "title", title: "mira@dev-staging: ~/app" },
    {
      type: "program_status",
      state: "blocked",
      kind: "question",
      app: "claude",
      message: "May it edit src/main.rs?",
    },
  ],
  p4: [
    {
      type: "disconnected",
      reason: "connection_lost",
      detail: "client_loop: send disconnect: Broken pipe",
    },
  ],
};

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

async function until(what: string, done: () => boolean): Promise<void> {
  const deadline = performance.now() + CHROME_START_TIMEOUT_MS;
  while (!done()) {
    if (performance.now() > deadline) throw new Error(`${what} never happened`);
    await sleep(POLL_MS);
  }
}

/** Has the fixture's sessions say what the header scenes have them say. */
function report(core: FakeCore, reports: Readonly<Record<string, readonly SessionEvent[]>>): void {
  for (const [pane, events] of Object.entries(reports)) {
    for (const event of events) core.event(pane, event);
  }
}

/** The header scenes' reports, and the end of the lost connection. */
async function headerStates(app: ShellApp, core: FakeCore): Promise<void> {
  report(core, REPORTS);
  core.exit("p4", { code: 255 });
  await until("the lost connection being shown", () => app.reports.get("p4").exit !== undefined);
}

/** Gives the first pane of the window `share` of its width. */
async function narrowFirst(app: ShellApp, share: number): Promise<void> {
  await app.client.layoutApply({ type: "resize", split: "s1", ratios: [share, WHOLE - share] });
  await until("the panes taking their new widths", () => {
    const first = document.querySelector<HTMLElement>('.pane-card[data-pane="p1"]');
    const area = first?.closest<HTMLElement>(".tab-area");
    if (!first || !area) return false;
    // The divided space is the tab's, less the gaps; a share of it is a little less
    // than the same share of the whole.
    return first.getBoundingClientRect().width <= (share / WHOLE) * area.clientWidth;
  });
}

/** What is wrong with how an element in a pane's header is shown, if anything. */
function shown(page: HTMLElement, pane: string, part: string, wanted: boolean): string[] {
  const element = page.querySelector<HTMLElement>(`.pane-card[data-pane="${pane}"] ${part}`);
  const is = element !== null && getComputedStyle(element).display !== "none";
  return is === wanted ? [] : [`${part} in ${pane} is ${is ? "shown" : "not shown"}`];
}

function said(page: HTMLElement, selector: string, text: string): string[] {
  const now = page.querySelector(selector)?.textContent ?? "";
  return now === text
    ? []
    : [`${selector} says ${JSON.stringify(now)}, not ${JSON.stringify(text)}`];
}

/** The profiles of the scene of a pane with a look of its own. */
const LOOK_PROFILES: ProfileList = {
  revision: 1,
  default: null,
  notes: [],
  looked: true,
  profiles: [
    {
      id: "light-box",
      name: "Light box",
      launcher: PWSH,
      theme: "lume-light",
      font_size: 18,
      hidden: false,
      origin: "user",
    },
  ],
  looks: { "lume-light": { scheme: "light", tokens: LIGHT_TOKENS } },
};

/** The scenes, in the order they are run. */
export const PANE_SCENES: readonly Scene[] = [
  {
    name: "pane-header-states",
    window: { tabs: [4], panes: FOUR },
    prepare: headerStates,
    check: (page) => [
      ...said(page, '.pane-card[data-pane="p1"] .pane-title', "pwsh"),
      ...said(page, '.pane-card[data-pane="p1"] .pane-cwd', "~\\src\\lume"),
      ...said(page, '.pane-card[data-pane="p1"] .pane-exit', "exit 3"),
      ...said(page, '.pane-card[data-pane="p2"] .pane-title', "tar"),
      ...said(page, '.pane-card[data-pane="p2"] .pane-cwd', "Ubuntu:/home/fixture/notes"),
      ...said(page, '.pane-card[data-pane="p3"] .pane-status-label', "Waiting on you"),
      ...said(page, '.pane-card[data-pane="p4"] .pane-ended-reason', "Connection lost"),
      ...shown(page, "p2", ".pane-running", true),
      ...shown(page, "p2", ".pane-activity", true),
      ...shown(page, "p1", ".pane-activity", false),
    ],
  },
  {
    name: "pane-header-narrow",
    window: { tabs: [4], panes: FOUR },
    prepare: async (app, core) => {
      await headerStates(app, core);
      core.event("p1", { type: "program_status", state: "working", app: "cargo" });
      await narrowFirst(app, NARROW_SHARE);
    },
    check: (page) => [
      ...shown(page, "p1", ".pane-cwd", false),
      ...shown(page, "p1", ".pane-status-label", false),
      ...shown(page, "p1", ".pane-status-mark", true),
      ...shown(page, "p3", ".pane-status-label", true),
    ],
  },
  {
    name: "pane-header-tiny",
    window: { tabs: [2], panes: FOUR },
    prepare: async (app, core) => {
      report(core, { p1: REPORTS["p1"] ?? [] });
      await narrowFirst(app, TINY_SHARE);
    },
    check: (page) => [
      ...shown(page, "p1", ".pane-icon", false),
      ...shown(page, "p1", ".pane-exit", false),
      ...shown(page, "p2", ".pane-icon", true),
    ],
  },
  {
    name: "tab-activity",
    window: {
      tabs: [2, 1, 3],
      titleBar: "custom",
      panes: { p5: { launcher: PWSH }, p6: { launcher: PWSH } },
    },
    prepare: async (app, core) => {
      report(core, {
        p2: [{ type: "bell" }],
        p3: [{ type: "activity" }],
        p4: [{ type: "foreground", pid: 40, name: "nvim" }],
        p5: [
          { type: "foreground", pid: 50, name: "lazygit.exe" },
          { type: "command_finished", exit: 1 },
        ],
        p6: [{ type: "foreground", pid: 60, name: "htop" }],
      });
      await until("the tabs taking what their panes gathered", () => {
        return (
          document.querySelectorAll('.tab-label[data-activity="attention-error"]').length === 1
        );
      });
    },
    check: (page) => {
      const tabs = [...page.querySelectorAll<HTMLElement>(".tab-label")];
      const activity = tabs.map((tab) => tab.dataset["activity"] ?? "");
      // The tab's own pictures, not the bright copy the moving mark lights up.
      const stacked = tabs[2]?.querySelectorAll(":scope > .tab-icons > .tab-icons-one").length ?? 0;
      const problems: string[] = [];
      if (activity.join(" ") !== "attention output attention-error") {
        problems.push(`the tabs gathered ${activity.join(", ")}`);
      }
      if (stacked !== 3) problems.push(`the third tab shows ${stacked} programs, not 3`);
      return problems;
    },
  },
  {
    name: "minimised-waiting",
    window: { tabs: [1], minimised: true },
    prepare: async (app, core) => {
      report(core, {
        p2: [
          { type: "foreground", pid: 70, name: "claude" },
          { type: "program_status", state: "blocked", kind: "permission" },
        ],
      });
      await until("the put-away pane saying it waits", () => {
        return document.querySelector('.pane-pill[data-waiting="true"]') !== null;
      });
    },
    check: (page) => said(page, ".pane-pill-name", "claude"),
  },
  {
    // The Activity list open: an agent waiting on the user, a command at work, and a
    // command that failed while the user looked elsewhere.
    name: "activity",
    window: { tabs: [1, 1, 1, 1], titleBar: "custom" },
    prepare: async (app, core) => {
      report(core, {
        p2: [
          { type: "foreground", pid: 70, name: "claude" },
          {
            type: "program_status",
            state: "blocked",
            kind: "permission",
            message: "Asks: apply migration 0042 to staging?",
          },
        ],
        p3: [{ type: "command_started", command: "cargo test -p billing" }],
        p4: [
          { type: "command_started", command: "npm run build" },
          { type: "command_finished", exit: 1 },
        ],
      });
      await until("the work being reported", () => app.reports.get("p4").activity !== "idle");
      document.querySelector<HTMLElement>('.top-tool[aria-haspopup="dialog"]')?.click();
      await until("the Activity list opening", () => document.querySelector(".activity") !== null);
    },
    check: (page) => {
      const rows = page.querySelectorAll(".activity-row").length;
      return rows === 3 ? [] : [`the Activity list has ${rows} rows, not 3`];
    },
  },
  {
    // The question asked before a tab whose agent waits on the user is closed from its
    // close button, hanging under it.
    name: "close-question",
    window: {
      tabs: [1, 1],
      titleBar: "custom",
      settings: { terminal: { "confirm-close": "always" } },
      commands: [
        ...FIXTURE_COMMANDS,
        { id: "lume.tab.close", title: "Close Tab", category: "Tab", runs: "core", palette: true },
      ],
    },
    prepare: async (app, core) => {
      report(core, {
        p2: [
          { type: "foreground", pid: 70, name: "claude" },
          { type: "program_status", state: "blocked", kind: "permission" },
        ],
      });
      await until("the agent being reported", () => app.reports.get("p2").statuses.length > 0);
      const tab = core.tabs[1] ?? "";
      app.commands.run("lume.tab.close", { tab });
      await until("the question being asked", () => {
        return document.querySelector(".close-question") !== null;
      });
    },
    check: (page) => [
      ...said(page, ".close-question h2", "Close this tab?"),
      ...said(page, ".close-why-text b", "claude"),
      ...(page.querySelector(".close-question[data-anchored]") === null
        ? ["the question does not hang under the close button"]
        : []),
    ],
  },
  {
    name: "pane-look",
    window: {
      tabs: [2],
      profiles: LOOK_PROFILES,
      panes: { p2: { launcher: PWSH, profile: "light-box" } },
    },
    prepare: async (app) => {
      await until("the profiles arriving", () => app.profiles.getState().list !== null);
      await until("the pane taking its profile's look", () => {
        const look = (pane: string) =>
          document.querySelector<HTMLElement>(`.pane-card[data-pane="${pane}"] .terminal`)?.style
            .backgroundColor ?? "";
        return look("p2") !== "" && look("p2") !== look("p1");
      });
    },
  },
];
