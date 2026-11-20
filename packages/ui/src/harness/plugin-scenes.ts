/**
 * The scenes of what plugins add to the window: meters and a button in the top bar,
 * an item's card and its menu, a plugin's menu among the application's and a section
 * of its own in another, a section in a tab's menu, a list panel, and a plugin's page
 * of settings with a row of it on one of the application's pages.
 *
 * What the core would send is written out here as the core works it out: the menus
 * with the plugins' nodes in their places, the items with their orders.
 */

import schemaText from "../../../../schemas/config.schema.json?raw";
import type { CommandSpec, MenuNodeData, UiContributions } from "../core/types.ts";
import type { FakeCoreOptions } from "./fake-core.ts";
import { FIXTURE_BINDINGS, FIXTURE_COMMANDS } from "./fixture-commands.ts";
import type { Scene } from "./scene.ts";
import { CHROME_START_TIMEOUT_MS } from "./tuning.ts";
import type { AddonEntry } from "../settings/addons.ts";

/** How often a scene looks again for what it waits for. */
const POLL_MS = 20;

async function until<T>(what: string, find: () => T | null | undefined): Promise<T> {
  const deadline = performance.now() + CHROME_START_TIMEOUT_MS;
  for (;;) {
    const found = find();
    if (found !== null && found !== undefined) return found;
    if (performance.now() > deadline) throw new Error(`${what} never appeared`);
    await new Promise((resolve) => window.setTimeout(resolve, POLL_MS));
  }
}

function pluginCommand(id: string, title: string): CommandSpec {
  return {
    id,
    title,
    category: id.startsWith("acme.meters.") ? "Meters" : "Tools",
    runs: "plugin",
    palette: true,
  };
}

/** The commands there are, with the plugins'. */
const COMMANDS: readonly CommandSpec[] = [
  ...FIXTURE_COMMANDS,
  pluginCommand("acme.meters.refresh", "Refresh meters"),
  pluginCommand("acme.meters.processes", "Show busiest processes"),
  pluginCommand("acme.tools.copy-name", "Copy tab name"),
  pluginCommand("acme.tools.jobs", "Show jobs"),
  pluginCommand("acme.tools.stop", "Stop job"),
  { id: "lume.panel.show", title: "Show Panel", category: "Panel", runs: "core", palette: false },
  { id: "lume.panel.close", title: "Close Panel", category: "Panel", runs: "core", palette: false },
];

/** A small drawing of a gear, which the plugin's manifest says takes the colour around it. */
const GEAR =
  "data:image/svg+xml;base64," +
  btoa(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="5" fill="none" stroke="black" stroke-width="2"/><circle cx="8" cy="8" r="1.5"/></svg>',
  );

const item = (key: string): MenuNodeData => ({ kind: "item", key: `lume:${key}` });
const section = (key: string, children: readonly MenuNodeData[]): MenuNodeData => ({
  kind: "section",
  key: `lume:${key}`,
  children,
});

/** What the core sends with two plugins installed: meters, and tools. */
export const PLUGIN_CONTRIBUTIONS: UiContributions = {
  revision: 3,
  plugins: [
    {
      id: "acme.meters",
      name: "Performance metrics",
      version: "1.2.0",
      ready: true,
      problem: null,
    },
    { id: "acme.tools", name: "Tools", version: "0.4.0", ready: true, problem: null },
  ],
  menus: {
    "menu.app": [
      { kind: "submenu", key: "lume:lume" },
      { kind: "submenu", key: "lume:view" },
      {
        kind: "submenu",
        key: "acme.tools:tools",
        plugin: "acme.tools",
        title: "Tools",
        children: [
          {
            kind: "item",
            key: "acme.tools:jobs-item",
            plugin: "acme.tools",
            title: "Show jobs",
            command: "acme.tools.jobs",
            icon: { image: "acme.tools:gear" },
          },
          {
            kind: "item",
            key: "acme.tools:refresh-item",
            plugin: "acme.tools",
            title: "Refresh meters",
            command: "acme.meters.refresh",
          },
        ],
      },
      { kind: "submenu", key: "lume:help" },
    ],
    "menu.app.view": [
      section("splits", [item("split-right"), item("split-down")]),
      section("zoom", [item("zoom-in"), item("zoom-out"), item("zoom-reset")]),
      {
        kind: "section",
        key: "acme.meters:meters",
        plugin: "acme.meters",
        title: "Meters",
        children: [
          {
            kind: "item",
            key: "acme.meters:refresh",
            plugin: "acme.meters",
            title: "Refresh meters",
            command: "acme.meters.refresh",
          },
          {
            kind: "item",
            key: "acme.meters:processes",
            plugin: "acme.meters",
            title: "Show busiest processes",
            command: "acme.meters.processes",
          },
        ],
      },
      section("panels", [item("panel-list")]),
    ],
    "menu.tab": [
      section("name", [item("rename"), item("color")]),
      section("group", [item("new-group"), item("to-group"), item("remove-from-group")]),
      section("position", [
        item("new-tab"),
        item("move-left"),
        item("move-right"),
        item("move-into"),
      ]),
      {
        kind: "section",
        key: "acme.tools:tab",
        plugin: "acme.tools",
        children: [
          {
            kind: "item",
            key: "acme.tools:copy-name",
            plugin: "acme.tools",
            title: "Copy tab name",
            command: "acme.tools.copy-name",
          },
        ],
      },
      section("closing", [item("lock"), item("close-left"), item("close-right"), item("close")]),
    ],
    "menu.status": [
      {
        kind: "submenu",
        key: "acme.meters:cpu-menu",
        plugin: "acme.meters",
        title: "Processor",
        children: [
          {
            kind: "item",
            key: "acme.meters:cpu-processes",
            plugin: "acme.meters",
            title: "Show busiest processes",
            command: "acme.meters.processes",
          },
          {
            kind: "item",
            key: "acme.meters:cpu-refresh",
            plugin: "acme.meters",
            title: "Refresh meters",
            command: "acme.meters.refresh",
          },
        ],
      },
    ],
  },
  status: [
    {
      id: "acme.tools.jobs",
      plugin: "acme.tools",
      slot: "topbar.left",
      kind: "badge",
      order: 0,
      command: "acme.tools.jobs",
      tooltip: "Jobs running",
    },
    {
      id: "acme.meters.cpu",
      plugin: "acme.meters",
      slot: "topbar.right",
      kind: "meter",
      order: 0,
      label: "CPU",
      menu: "acme.meters:cpu-menu",
      slot_setting: { key: "place", slots: ["statusbar.right", "topbar.right"] },
    },
    {
      id: "acme.meters.memory",
      plugin: "acme.meters",
      slot: "topbar.right",
      kind: "meter",
      order: 1,
      label: "RAM",
      slot_setting: { key: "place", slots: ["statusbar.right", "topbar.right"] },
    },
    {
      id: "acme.meters.gpu",
      plugin: "acme.meters",
      slot: "topbar.right",
      kind: "meter",
      order: 2,
      label: "GPU",
      slot_setting: { key: "place", slots: ["statusbar.right", "topbar.right"] },
    },
    {
      id: "acme.meters.network",
      plugin: "acme.meters",
      slot: "topbar.right",
      kind: "sparkline",
      order: 3,
      label: "NET",
      slot_setting: { key: "place", slots: ["statusbar.right", "topbar.right"] },
    },
  ],
  panels: [
    {
      id: "acme.tools.jobs-panel",
      plugin: "acme.tools",
      view: "jobs-panel",
      title: "Jobs",
      kind: "list",
      area: "right",
      order: 0,
      icon: { image: "acme.tools:gear" },
    },
  ],
  pane_views: [],
  openers: [],
  images: { "acme.tools:gear": { hash: "gear", tint: true } },
  motions: [],
  cursor_effects: [],
  values: [
    {
      plugin: "acme.tools",
      item: "acme.tools.jobs",
      pane: null,
      value: { text: "3", tone: "warning" },
    },
    {
      plugin: "acme.meters",
      item: "acme.meters.cpu",
      pane: null,
      value: {
        value: 0.31,
        text: "31%",
        color: "color.accent",
        card: [
          { label: "All cores", value: "31%", points: [0.22, 0.3, 0.28, 0.41, 0.35, 0.31] },
          { label: "Busiest core", value: "74%", tone: "warning" },
          { label: "Processes", value: "312" },
        ],
      },
    },
    {
      plugin: "acme.meters",
      item: "acme.meters.memory",
      pane: null,
      value: { value: 0.35, text: "11.2 / 32G", color: "color.status.info" },
    },
    {
      plugin: "acme.meters",
      item: "acme.meters.gpu",
      pane: null,
      value: { value: 0.08, text: "8%", color: "color.tab.purple" },
    },
    {
      plugin: "acme.meters",
      item: "acme.meters.network",
      pane: null,
      value: {
        points: [0.4, 1.2, 0.9, 2.6, 1.8, 2.1],
        text: "↓ 2.1 MB/s",
        color: "color.tab.teal",
      },
    },
  ],
  rows: {
    "acme.tools.jobs-panel": [
      {
        id: "build",
        label: "build",
        detail: "cargo build --release",
        badge: "2",
        expanded: true,
        children: [
          {
            id: "compile",
            label: "Compiling lume-app",
            detail: "41 s",
            tone: "info",
            actions: [{ command: "acme.tools.stop", title: "Stop" }],
          },
          { id: "link", label: "Linking", detail: "waiting" },
        ],
      },
      { id: "tests", label: "tests", detail: "cargo xtask test", tone: "success", badge: "ok" },
      { id: "lint", label: "lint", detail: "failed after 12 s", tone: "danger", badge: "1" },
    ],
  },
  contexts: {},
};

const WITH_PLUGINS: FakeCoreOptions = {
  tabs: [2],
  commands: COMMANDS,
  bindings: FIXTURE_BINDINGS,
  titleBar: "custom",
  contributions: PLUGIN_CONTRIBUTIONS,
  assets: { gear: GEAR },
};

/** The same, with a row in the panel whose rows are given when it is opened. */
const WITH_LAZY_ROWS: FakeCoreOptions = {
  ...WITH_PLUGINS,
  contributions: {
    ...PLUGIN_CONTRIBUTIONS,
    rows: {
      "acme.tools.jobs-panel": [
        ...(PLUGIN_CONTRIBUTIONS.rows["acme.tools.jobs-panel"] ?? []),
        { id: "artifacts", label: "artifacts", detail: "target/release", lazy: true },
        { id: "cache", label: "cache", detail: "never answers", lazy: true },
      ],
    },
  },
  lazyRows: {
    "acme.tools.jobs-panel": {
      artifacts: [
        { id: "lume", label: "lume.exe", detail: "9.4 MB" },
        { id: "gui", label: "lume-gui.exe", detail: "21 MB" },
      ],
    },
  },
  pluginPanel: {
    id: "acme.tools.jobs-panel",
    plugin: "acme.tools",
    view: "jobs-panel",
    area: "right",
  },
};

/** The same, with the meters' setting putting them in the status bar. */
const WITH_METERS_BELOW: FakeCoreOptions = {
  ...WITH_PLUGINS,
  settings: { plugins: { "acme.meters": { place: "statusbar.right" } } },
};

/** What is wrong with `what`, which should be over the item `under`. */
function over(what: Element | null, under: Element | null, name: string): string[] {
  if (!what || !under) return [`${name} is not on show`];
  const box = what.getBoundingClientRect();
  return box.bottom <= under.getBoundingClientRect().top
    ? []
    : [`${name} is not over its item, at ${String(box.top)} to ${String(box.bottom)}`];
}

/** Waits for the plugins' items to be in the top bar, and gives the one asked for. */
function statusItem(id: string): Promise<HTMLElement> {
  return until(`the status item ${id}`, () =>
    document.querySelector<HTMLElement>(`[data-status="${id}"]`),
  );
}

function rowsOf(menu: Element): string[] {
  return [...menu.querySelectorAll(":scope > .menu-item .menu-label, :scope > .menu-heading")].map(
    (row) => row.textContent ?? "",
  );
}

function compare(found: readonly string[], wanted: readonly string[], what: string): string[] {
  return found.join("|") === wanted.join("|")
    ? []
    : [`${what} has ${JSON.stringify(found)}, not ${JSON.stringify(wanted)}`];
}

/** Opens one of the menus of the application's menu bar, by its name. */
async function openAppMenu(name: string): Promise<Element> {
  const button = await until("the application's menu button", () =>
    document.querySelector<HTMLButtonElement>(".app-menu-button"),
  );
  button.click();
  const named = await until(`the menu ${name} in the bar`, () =>
    [...document.querySelectorAll<HTMLButtonElement>(".app-menu-name")].find(
      (candidate) => candidate.textContent === name,
    ),
  );
  named.click();
  return until(`the menu ${name}`, () =>
    [...document.querySelectorAll(".app-menu-drop > .menu")].find(
      (menu) => menu.getAttribute("aria-label") === name,
    ),
  );
}

/** The fixture's plugins, one of whose panels shows its own page. */
const WITH_FRAME_PANEL: FakeCoreOptions = {
  ...WITH_PLUGINS,
  contributions: {
    ...PLUGIN_CONTRIBUTIONS,
    panels: [
      ...PLUGIN_CONTRIBUTIONS.panels,
      {
        id: "acme.tools.preview",
        plugin: "acme.tools",
        view: "preview",
        title: "Preview",
        kind: "frame",
        area: "right",
        order: 1,
        entry: "ui/preview.html",
      },
    ],
  },
  pluginPanel: { id: "acme.tools.preview", plugin: "acme.tools", view: "preview", area: "right" },
};

/** The fixture's plugins, one of which has a view for panes, shown beside a terminal. */
const WITH_FRAME_PANE: FakeCoreOptions = {
  ...WITH_PLUGINS,
  contributions: {
    ...PLUGIN_CONTRIBUTIONS,
    pane_views: [
      {
        id: "acme.tools.report",
        plugin: "acme.tools",
        view: "report",
        title: "Report",
        entry: "ui/report.html",
        persist: true,
      },
    ],
  },
  pluginPane: { pane: "p2", plugin: "acme.tools", view: "report" },
};

export const PLUGIN_SCENES: readonly Scene[] = [
  {
    // A plugin's own page in a pane beside a terminal, while it loads: the pane has the
    // header of any pane, called by the view's name until it is given a title, and no
    // terminal in it.
    name: "plugin-frame-pane",
    window: WITH_FRAME_PANE,
    prepare: async () => {
      await until("the pane's frame", () => document.querySelector(".pane-plugin iframe"));
    },
    check: (page) => {
      const card = page.querySelector(".pane-plugin")?.closest(".pane-card");
      const frame = card?.querySelector<HTMLIFrameElement>(".plugin-frame iframe");
      if (!card || !frame) return ["the pane has no frame"];
      const problems: string[] = [];
      if (frame.getAttribute("sandbox") !== "allow-scripts") {
        problems.push(`the frame's sandbox is ${frame.getAttribute("sandbox")}`);
      }
      if (!frame.src.endsWith("/acme.tools/ui/report.html")) {
        problems.push(`the frame shows ${frame.src}`);
      }
      if (card.querySelector(".pane-title")?.textContent !== "report") {
        problems.push(`the pane is called ${card.querySelector(".pane-title")?.textContent}`);
      }
      if (card.querySelector(".terminal")) {
        problems.push("a terminal is drawn in the view's pane");
      }
      if (page.querySelectorAll(".pane-card").length !== 2) {
        problems.push(`there are ${page.querySelectorAll(".pane-card").length} panes`);
      }
      return problems;
    },
  },
  {
    // A plugin's own page in a panel, while it loads: the stand-in never answers it.
    name: "plugin-frame-panel",
    window: WITH_FRAME_PANEL,
    prepare: async () => {
      await until("the panel's frame", () => document.querySelector(".plugin-frame iframe"));
    },
    check: (page) => {
      const frame = page.querySelector<HTMLIFrameElement>(".plugin-frame iframe");
      if (!frame) return ["the panel has no frame"];
      const problems: string[] = [];
      if (frame.getAttribute("sandbox") !== "allow-scripts") {
        problems.push(`the frame's sandbox is ${frame.getAttribute("sandbox")}`);
      }
      if (!frame.src.endsWith("/acme.tools/ui/preview.html")) {
        problems.push(`the frame shows ${frame.src}`);
      }
      if (!page.querySelector('.plugin-frame[data-shown="loading"] .plugin-frame-spinner')) {
        problems.push("the frame does not say it is loading");
      }
      return problems;
    },
  },
  {
    // A view that could not be opened says so, with a way to try again.
    name: "plugin-frame-stopped",
    window: { ...WITH_FRAME_PANEL, refuseViews: true },
    prepare: async () => {
      await until("the stopped view", () =>
        document.querySelector('.plugin-frame[data-shown="stopped"]'),
      );
    },
    check: (page) => {
      const note = page.querySelector('.plugin-frame[data-shown="stopped"] [role="alert"]');
      if (!note) return ["the view does not say it stopped"];
      const problems: string[] = [];
      if (!note.textContent?.includes("Preview stopped")) {
        problems.push(`the note says ${note.textContent}`);
      }
      if (page.querySelector(".plugin-frame iframe")) problems.push("a frame is still there");
      if (!page.querySelector(".plugin-frame-again"))
        problems.push("there is no way to load it again");
      return problems;
    },
  },
  {
    name: "plugin-status",
    window: WITH_PLUGINS,
    prepare: async () => {
      await statusItem("acme.meters.network");
    },
    check: (page) => {
      const right = [...page.querySelectorAll('[data-slot="topbar.right"] .status-label')].map(
        (label) => label.textContent ?? "",
      );
      const problems = compare(right, ["CPU", "RAM", "GPU", "NET"], "the right of the top bar");
      if (!page.querySelector('button.status-button[data-status="acme.tools.jobs"]')) {
        problems.push("the plugin's badge that runs a command is not a button");
      }
      return problems;
    },
  },
  {
    name: "plugin-status-card",
    window: WITH_PLUGINS,
    prepare: async () => {
      const cpu = await statusItem("acme.meters.cpu");
      cpu.dispatchEvent(new PointerEvent("pointerenter"));
      cpu.dispatchEvent(new PointerEvent("pointerover", { bubbles: true }));
      await until("the CPU's card", () => document.querySelector(".status-card"));
    },
    check: (page) => {
      const card = page.querySelector(".status-card");
      if (!card) return ["the CPU's card is not on show"];
      const rows = [...card.querySelectorAll(".status-card-label")].map(
        (label) => label.textContent ?? "",
      );
      return compare(rows, ["All cores", "Busiest core", "Processes"], "the CPU's card");
    },
  },
  {
    name: "plugin-status-menu",
    window: WITH_PLUGINS,
    prepare: async () => {
      const cpu = await statusItem("acme.meters.cpu");
      cpu.click();
      await until("the CPU's menu", () => document.querySelector(".status-menu .menu"));
    },
    check: (page) => {
      const menu = page.querySelector(".status-menu .menu");
      if (!menu) return ["the CPU's menu is not open"];
      return compare(rowsOf(menu), ["Show busiest processes", "Refresh meters"], "the CPU's menu");
    },
  },
  {
    name: "plugin-status-bar",
    window: WITH_METERS_BELOW,
    prepare: async () => {
      await statusItem("acme.meters.network");
    },
    check: (page) => {
      const below = [
        ...page.querySelectorAll('.status-bar [data-slot="statusbar.right"] .status-label'),
      ].map((label) => label.textContent ?? "");
      const problems = compare(below, ["CPU", "RAM", "GPU", "NET"], "the right of the status bar");
      if (page.querySelector('[data-slot="topbar.right"] [data-plugin="acme.meters"]')) {
        problems.push("a meter is still in the top bar");
      }
      const bar = page.querySelector(".status-bar")?.getBoundingClientRect();
      if (!bar || Math.round(bar.bottom) !== Math.round(page.getBoundingClientRect().bottom)) {
        problems.push("the status bar is not along the bottom of the window");
      }
      return problems;
    },
  },
  {
    name: "plugin-status-bar-card",
    window: WITH_METERS_BELOW,
    prepare: async () => {
      const cpu = await statusItem("acme.meters.cpu");
      cpu.dispatchEvent(new PointerEvent("pointerenter"));
      cpu.dispatchEvent(new PointerEvent("pointerover", { bubbles: true }));
      await until("the CPU's card", () => document.querySelector(".status-card"));
    },
    check: (page) =>
      over(
        page.querySelector(".status-card"),
        page.querySelector('[data-status="acme.meters.cpu"]'),
        "the CPU's card",
      ),
  },
  {
    name: "plugin-status-bar-menu",
    window: WITH_METERS_BELOW,
    prepare: async () => {
      const cpu = await statusItem("acme.meters.cpu");
      cpu.click();
      await until("the CPU's menu", () => document.querySelector(".status-menu .menu"));
    },
    check: (page) => {
      const menu = page.querySelector(".status-menu .menu");
      return [
        ...over(menu, page.querySelector('[data-status="acme.meters.cpu"]'), "the CPU's menu"),
        ...(menu
          ? compare(rowsOf(menu), ["Show busiest processes", "Refresh meters"], "the CPU's menu")
          : []),
      ];
    },
  },
  {
    name: "plugin-app-menu",
    window: WITH_PLUGINS,
    prepare: async () => {
      await openAppMenu("View");
    },
    check: (page) => {
      const names = [...page.querySelectorAll(".app-menu-name")].map((name) => name.textContent);
      const problems = compare(
        names.map((name) => name ?? ""),
        ["View", "Tools", "Help"],
        "the menu bar",
      );
      const menu = page.querySelector(".app-menu-drop > .menu");
      if (!menu) return [...problems, "the View menu is not open"];
      return [
        ...problems,
        ...compare(
          rowsOf(menu),
          // The fixture has some of the application's commands, and the menu shows those.
          [
            "Split right",
            "Zoom in",
            "Meters",
            "Refresh meters",
            "Show busiest processes",
            "Show Jobs",
          ],
          "the View menu",
        ),
      ];
    },
  },
  {
    name: "plugin-tools-menu",
    window: WITH_PLUGINS,
    prepare: async () => {
      await openAppMenu("Tools");
    },
    check: (page) => {
      const menu = page.querySelector(".app-menu-drop > .menu");
      if (!menu) return ["the Tools menu is not open"];
      return compare(rowsOf(menu), ["Show jobs", "Refresh meters"], "the Tools menu");
    },
  },
  {
    name: "plugin-tab-menu",
    window: WITH_PLUGINS,
    prepare: async () => {
      const tab = await until("the first tab", () =>
        document.querySelector<HTMLElement>('.tab-strip [data-tab-id="t1"]'),
      );
      const box = tab.getBoundingClientRect();
      tab.dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          cancelable: true,
          button: 2,
          clientX: box.left + box.width / 2,
          clientY: box.top + box.height / 2,
        }),
      );
      await until("the tab's menu", () => document.querySelector(".tab-menu .menu"));
    },
    check: (page) => {
      const menu = page.querySelector(".tab-menu .menu");
      if (!menu) return ["the tab's menu is not open"];
      const rows = rowsOf(menu);
      const at = rows.indexOf("Copy tab name");
      return at > rows.indexOf("Move into another tab…") && at < rows.indexOf("Lock session")
        ? []
        : [`the plugin's item is not between the tab's moves and its closing: ${rows.join(", ")}`];
    },
  },
  {
    name: "plugin-ask-permission",
    window: WITH_PLUGINS,
    prepare: async (app) => {
      void app.prompts.askPlugin({
        kind: "permission",
        plugin: { id: "acme.tools", name: "Tools" },
        permission: "process.run",
        what: "Run these programs, with any arguments: git",
        warning:
          "A program such as a shell or git can do anything you can, so running it is close to trusting this plugin with your account.",
        reason: "to show which files changed in the workspace",
      });
      await until("the question", () => document.querySelector(".prompt"));
    },
    check: (page) => {
      const dialog = page.ownerDocument.querySelector('[role="alertdialog"]');
      if (!dialog) return ["the question is not on show"];
      const problems: string[] = [];
      if (!dialog.textContent?.includes("Tools says:"))
        problems.push("the plugin's words are not under its name");
      if (dialog.querySelector("button")?.textContent !== "Don't allow") {
        problems.push("the cautious answer does not come first");
      }
      return problems;
    },
  },
  {
    name: "plugin-ask-each",
    window: WITH_PLUGINS,
    prepare: async (app) => {
      void app.prompts.askPlugin({
        kind: "each",
        plugin: { id: "acme.tools", name: "Tools" },
        permission: "panes.write",
        text: "cargo test --workspace\r",
        place: "pane p1, Terminal",
      });
      await until("the question", () => document.querySelector(".prompt"));
    },
    check: (page) => {
      const shown = page.ownerDocument.querySelector(".prompt-preview")?.textContent ?? "";
      return shown.startsWith("cargo test --workspace") ? [] : ["the exact text is not shown"];
    },
  },
  {
    name: "plugin-pick",
    window: WITH_PLUGINS,
    prepare: async (app) => {
      void app.prompts.askPlugin({
        kind: "pick",
        plugin: { id: "acme.tools", name: "Tools" },
        title: "Run a job",
        placeholder: "Type to find a job",
        items: [
          { id: "build", label: "build", detail: "cargo build --release" },
          { id: "test", label: "test", detail: "cargo xtask test" },
          { id: "lint", label: "lint", detail: "cargo xtask lint" },
        ],
      });
      await until("the choice", () => document.querySelector(".plugin-pick"));
    },
    check: (page) => {
      const rows = [...page.ownerDocument.querySelectorAll(".plugin-pick .palette-row-text")].map(
        (row) => row.textContent ?? "",
      );
      return compare(rows, ["build", "test", "lint"], "the choice");
    },
  },
  {
    name: "plugin-list-panel",
    window: {
      ...WITH_PLUGINS,
      pluginPanel: {
        id: "acme.tools.jobs-panel",
        plugin: "acme.tools",
        view: "jobs-panel",
        area: "right",
      },
    },
    prepare: async () => {
      await until("the panel's rows", () => document.querySelector(".list-row"));
    },
    check: (page) => {
      const rows = [...page.querySelectorAll(".list-row .list-row-label")].map(
        (label) => label.textContent ?? "",
      );
      return compare(
        rows,
        ["build", "Compiling lume-app", "Linking", "tests", "lint"],
        "the panel",
      );
    },
  },
  {
    name: "plugin-list-panel-lazy",
    window: WITH_LAZY_ROWS,
    prepare: async () => {
      const open = async (label: string) => {
        const row = await until(`the row ${label}`, () =>
          [...document.querySelectorAll<HTMLElement>(".list-row")].find(
            (one) => one.querySelector(".list-row-label")?.textContent === label,
          ),
        );
        row.querySelector<HTMLElement>(".list-row-chevron")?.click();
      };
      // One row's rows come when it is opened; the other's never do, and it says so
      // while it waits.
      await open("artifacts");
      await until("the rows that were asked for", () =>
        [...document.querySelectorAll(".list-row-label")].find(
          (label) => label.textContent === "lume-gui.exe",
        ),
      );
      await open("cache");
      await until("the line that says rows are on their way", () =>
        document.querySelector(".list-row-note"),
      );
    },
    check: (page) => {
      const rows = [...page.querySelectorAll(".list-row .list-row-label")].map(
        (label) => label.textContent ?? "",
      );
      return compare(
        rows,
        [
          "build",
          "Compiling lume-app",
          "Linking",
          "tests",
          "lint",
          "artifacts",
          "lume.exe",
          "lume-gui.exe",
          "cache",
          "Loading…",
        ],
        "the panel",
      );
    },
  },
];

/** The settings' schema with the meters' table and page, as the core composes it. */
function pluginSchema(): unknown {
  const schema = JSON.parse(schemaText) as {
    properties: { plugins: { properties?: Record<string, unknown> } };
    "x-lume-pages": unknown[];
  };
  const page = "addon:acme.meters";
  const from = "Performance metrics";
  const meter = (label: string, order: number, fallback: boolean, help?: string) => ({
    type: "boolean",
    default: fallback,
    "x-lume-settings": {
      page,
      group: "Status bar meters",
      order,
      label,
      control: "switch",
      plugin: from,
      ...(help === undefined ? {} : { help }),
    },
  });
  schema.properties.plugins.properties = {
    ...schema.properties.plugins.properties,
    "acme.meters": {
      type: "object",
      additionalProperties: false,
      properties: {
        place: {
          type: "string",
          default: "topbar.right",
          enum: ["statusbar.right", "topbar.right"],
          "x-lume-settings": {
            page,
            group: "Status bar meters",
            order: 5,
            label: "Show in",
            control: "segmented",
            plugin: from,
            options: [
              { value: "statusbar.right", label: "Status bar" },
              { value: "topbar.right", label: "Top bar" },
            ],
          },
        },
        cpu: meter("CPU", 10, true),
        memory: meter("Memory", 20, true),
        network: meter("Network", 30, false),
        gpu: meter("GPU", 40, false, "Shown where the graphics card can be read"),
        rate: {
          type: "integer",
          default: 2,
          enum: [1, 2, 5],
          "x-lume-settings": {
            page,
            group: "Sampling",
            order: 10,
            label: "Refresh every",
            control: "select",
            plugin: from,
            options: [
              { value: 1, label: "1 s" },
              { value: 2, label: "2 s" },
              { value: 5, label: "5 s" },
            ],
          },
        },
        "pane-headers": {
          type: "boolean",
          default: false,
          "x-lume-settings": {
            page: "appearance",
            group: "Panes",
            order: 1000,
            label: "Meters in pane headers",
            help: "Each pane's header shows what its programs use",
            control: "switch",
            plugin: from,
          },
        },
      },
    },
  };
  schema["x-lume-pages"] = [
    ...schema["x-lume-pages"],
    {
      id: page,
      label: from,
      icon: "plugin",
      group: "Add-ons",
      blurb: "Meters in the status bar or the top bar for this machine.",
      groups: ["Status bar meters", "Sampling"],
      meta: "v1.2.0",
      plugin: "acme.meters",
    },
  ];
  return schema;
}

/** The plugins the settings window lists: the meters, a tool that uses another, and
 *  one whose files changed. */
export const FIXTURE_ADDONS: readonly AddonEntry[] = [
  {
    id: "acme.meters",
    manifestId: "acme.meters",
    name: "Performance metrics",
    version: "1.2.0",
    description: "Meters for this machine in the status bar.",
    enabled: true,
    state: { ready: true, reason: null },
    modified: false,
    missing: false,
    source: "D:/plugins/metrics",
    sourceKind: "folder",
    commit: null,
    dev: false,
    dependencies: [],
    permissions: [
      {
        what: "See how busy the processor, memory, graphics card and network are",
        warning: null,
      },
    ],
    later: [],
    health: { running: true, starts: 1, breaches: 0, lastProblem: null, warnings: [] },
    log: ["info: reading every 2 s", "info: the graphics card cannot be read here"],
    audit: [
      {
        time: "2026-11-19T09:14:02Z",
        capability: "system.metrics",
        scope: "",
        outcome: "allowed",
      },
    ],
  },
  {
    id: "acme.tools",
    manifestId: "acme.tools",
    name: "Tools",
    version: "0.4.0",
    description: "Jobs, and the commands to run them.",
    enabled: true,
    state: { ready: true, reason: null },
    modified: false,
    missing: false,
    source: "https://github.com/acme/tools.git#v2 (plugins/tools)",
    sourceKind: "git",
    commit: "4f9d2c81a07be35d6c1e9a0b7f2d4c6e8a1b3d5f",
    dev: false,
    dependencies: [{ id: "acme.core", optional: false, bound: "acme.core", rebound: false }],
    permissions: [
      {
        what: "Run these programs, with any arguments: cargo, git",
        warning:
          "A program such as a shell or git can do anything you can, so running it is close to trusting this plugin with your account.",
      },
      { what: "Read files in: ${workspace}/**", warning: null },
    ],
    later: ["Read what you copy"],
    health: {
      running: false,
      starts: 3,
      breaches: 1,
      lastProblem: "it ran for longer than 100 ms without a break",
      warnings: [],
    },
    log: [],
    audit: [
      { time: "2026-11-19T09:20:41Z", capability: "process.run", scope: "git", outcome: "allowed" },
      {
        time: "2026-11-19T09:21:07Z",
        capability: "fs.read",
        scope: "C:/Users/me/.ssh/id_ed25519",
        outcome: "denied",
      },
    ],
  },
  {
    id: "acme.core",
    manifestId: "acme.core",
    name: "Core tools",
    version: "2.0.1",
    description: null,
    enabled: false,
    state: { ready: false, reason: "it is turned off" },
    modified: true,
    missing: false,
    source: "D:/plugins/core",
    sourceKind: "folder",
    commit: null,
    dev: false,
    dependencies: [],
    permissions: [],
    later: [],
    health: { running: false, starts: 0, breaches: 0, lastProblem: null, warnings: [] },
    log: [],
    audit: [],
  },
];

/** The settings window with the meters installed, and two of them set. */
export const PLUGIN_SETTINGS: FakeCoreOptions = {
  titleBar: "custom",
  schema: pluginSchema(),
  settings: { plugins: { "acme.meters": { network: true, rate: 1 } } },
  addons: FIXTURE_ADDONS,
};

/** The settings scenes of plugins: their page, and their rows on the application's. */
export const PLUGIN_SETTINGS_SCENES: readonly {
  readonly name: string;
  readonly page: string;
  readonly window: FakeCoreOptions;
}[] = [
  { name: "settings-plugin-page", page: "addon:acme.meters", window: PLUGIN_SETTINGS },
  { name: "settings-plugin-rows", page: "appearance", window: PLUGIN_SETTINGS },
  { name: "settings-addons", page: "addons", window: PLUGIN_SETTINGS },
  { name: "settings-addon-health", page: "addon:acme.tools", window: PLUGIN_SETTINGS },
];
