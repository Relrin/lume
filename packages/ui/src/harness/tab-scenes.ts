/**
 * The scenes of the window's tabs: tabs with colours and a lock, tabs in groups with
 * one group folded, the menu of a tab and its second step, the editor of a group, the
 * menu of groups that were kept when they were closed, and a row that holds more tabs
 * than fit, with the list of them all.
 */

import type { CommandSpec } from "../core/types.ts";
import type { ShellApp } from "../shell/boot.ts";
import type { FakeCoreOptions } from "./fake-core.ts";
import { FIXTURE_BINDINGS, FIXTURE_COMMANDS } from "./fixture-commands.ts";
import type { Scene } from "./scene.ts";
import { CHROME_START_TIMEOUT_MS } from "./tuning.ts";

/** How often a scene looks again for what it waits for. */
const POLL_MS = 20;

/** A command about tabs, as the core lists it. */
function tabCommand(id: string, title: string, palette = false): CommandSpec {
  return {
    id,
    title,
    category: id.startsWith("lume.group.") ? "Group" : "Tab",
    runs: "core",
    palette,
  };
}

/** The commands of the tab menu and the group editor, besides the fixture's own. */
const TAB_COMMANDS: readonly CommandSpec[] = [
  tabCommand("lume.tab.close", "Close Tab", true),
  tabCommand("lume.tab.rename", "Rename Tab"),
  tabCommand("lume.tab.color", "Tab Colour"),
  tabCommand("lume.tab.lock", "Lock Tab", true),
  tabCommand("lume.tab.mute", "Mute or Unmute Tab", true),
  tabCommand("lume.group.mute", "Mute or Unmute Group", true),
  tabCommand("lume.tab.move-left", "Move Tab Left", true),
  tabCommand("lume.tab.move-right", "Move Tab Right", true),
  tabCommand("lume.tab.close-left", "Close Tabs to the Left", true),
  tabCommand("lume.tab.close-right", "Close Tabs to the Right", true),
  tabCommand("lume.tab.merge", "Move Tab into Another"),
  tabCommand("lume.group.new", "New Group", true),
  tabCommand("lume.group.add", "Add Tab to Group"),
  tabCommand("lume.group.remove", "Remove Tab from Group", true),
  tabCommand("lume.group.set", "Change Group"),
  tabCommand("lume.group.toggle", "Fold Group", true),
  tabCommand("lume.group.ungroup", "Ungroup", true),
  tabCommand("lume.group.close", "Close Group", true),
  tabCommand("lume.group.reopen", "Reopen Group"),
  {
    id: "lume.tab.rename-start",
    title: "Rename Tab…",
    category: "Tab",
    runs: "page",
    palette: true,
  },
];

const COMMANDS: readonly CommandSpec[] = [
  ...FIXTURE_COMMANDS.filter((spec) => !TAB_COMMANDS.some((tab) => tab.id === spec.id)),
  ...TAB_COMMANDS,
];

/** Five tabs of one pane: two in a group, one alone, two in another group. */
const GROUPED: FakeCoreOptions = {
  tabs: [1, 1, 1, 1, 1],
  commands: COMMANDS,
  bindings: FIXTURE_BINDINGS,
  titleBar: "custom",
  tabLooks: {
    t1: { title: "Fix tax rounding" },
    t2: { title: "cargo test" },
    t3: { title: "TODO.md", color: "purple" },
    t4: { title: "terraform apply", locked: true },
    t5: { title: "Processes" },
  },
  groups: [
    { tabs: [0, 1], name: "Billing", color: "teal", icon: "bug", kept: true },
    { tabs: [3, 4], name: "Infra", color: "blue", icon: "server" },
  ],
};

/** How many tabs the crowded row holds: more than fit in the window the scenes run in. */
const MANY = 16;

/** What the crowded row's tabs are called. */
const MANY_TITLES: readonly string[] = [
  "Fix tax rounding",
  "cargo test",
  "TODO.md",
  "terraform apply",
  "Processes",
  "api logs",
  "deploy staging",
  "review notes",
  "migrations",
  "docker compose",
  "benchmarks",
  "release checklist",
  "ssh bastion",
  "queue worker",
  "storybook",
  "scratch",
];

/** A row of more tabs than fit, the last of them on show. */
const CROWDED: FakeCoreOptions = {
  tabs: Array.from({ length: MANY }, () => 1),
  commands: COMMANDS,
  bindings: FIXTURE_BINDINGS,
  titleBar: "custom",
  activeTab: `t${MANY}`,
  tabLooks: Object.fromEntries(MANY_TITLES.map((title, index) => [`t${index + 1}`, { title }])),
};

/** The width of the fade at one end of the row of tabs, as the page set it. */
function fadeOf(list: HTMLElement, end: "start" | "end"): string {
  return list.style.getPropertyValue(`--tab-fade-${end}`);
}

/** Waits for the crowded row to have brought its last tab, the one on show, into sight. */
async function crowdedSettled(): Promise<HTMLElement> {
  const list = await until("the row of tabs holding more than fits", () =>
    document.querySelector<HTMLElement>('.tab-strip > .tab-list[data-over="true"]'),
  );
  await until("the row reaching its end, and its fades following", () =>
    fadeOf(list, "start") !== "0px" && fadeOf(list, "end") === "0px" ? true : null,
  );
  return list;
}

async function until<T>(what: string, find: () => T | null | undefined): Promise<T> {
  const deadline = performance.now() + CHROME_START_TIMEOUT_MS;
  for (;;) {
    const found = find();
    if (found !== null && found !== undefined) return found;
    if (performance.now() > deadline) throw new Error(`${what} never happened`);
    await new Promise((resolve) => window.setTimeout(resolve, POLL_MS));
  }
}

/** Presses something with the other button, as a person with a pointer does. */
function rightClick(element: Element): void {
  const box = element.getBoundingClientRect();
  element.dispatchEvent(
    new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      button: 2,
      clientX: box.left + box.width / 2,
      clientY: box.top + box.height / 2,
    }),
  );
}

/** Waits for the row of tabs to have a tab, by its id. */
function tabOf(tab: string): Promise<HTMLElement> {
  return until(`tab ${tab} on show`, () =>
    document.querySelector<HTMLElement>(`.tab-strip [data-tab-id="${tab}"]`),
  );
}

/** Opens the menu of a tab, and waits for it. */
async function openTabMenu(tab: string): Promise<HTMLElement> {
  rightClick(await tabOf(tab));
  return until("the tab's menu", () => document.querySelector<HTMLElement>(".tab-menu .menu"));
}

/** The labels of a menu's rows, headings and notes in order. */
function rowsOf(menu: Element): string[] {
  return [
    ...menu.querySelectorAll(
      ":scope > .menu-item, :scope > .menu-heading, :scope > .menu-swatches",
    ),
  ].map((row) => (row.querySelector(".menu-label") ?? row).textContent?.trim() ?? "");
}

/** Says what differs between the rows found and the rows wanted. */
function compare(found: readonly string[], wanted: readonly string[], what: string): string[] {
  return JSON.stringify(found) === JSON.stringify(wanted)
    ? []
    : [`${what} lists ${JSON.stringify(found)}, not ${JSON.stringify(wanted)}`];
}

/** Waits for the page to have the shelf the scene began with. */
function shelfArrived(app: ShellApp): Promise<boolean> {
  return until("the kept groups arriving", () =>
    app.layout.getState().shelf.length > 0 ? true : null,
  );
}

/** The scenes of the tabs, in the order they are run. */
export const TAB_SCENES: readonly Scene[] = [
  {
    name: "tab-colors",
    window: {
      tabs: [1, 1, 1, 1],
      titleBar: "custom",
      commands: COMMANDS,
      bindings: FIXTURE_BINDINGS,
      activeTab: "t2",
      tabLooks: {
        t1: { title: "api", color: "green" },
        t2: { title: "deploy", color: "orange" },
        t3: { title: "notes", locked: true },
        t4: { title: "logs", color: "pink", locked: true },
      },
    },
    check: (page) => {
      const problems: string[] = [];
      const locks = page.querySelectorAll(".tab-strip .tab-lock").length;
      if (locks !== 2) problems.push(`the row shows ${locks} locks, not 2`);
      const closes = page.querySelectorAll(".tab-strip .tab-close").length;
      if (closes !== 2) problems.push(`the row shows ${closes} close buttons, not 2`);
      return problems;
    },
  },
  {
    name: "tab-groups",
    window: GROUPED,
    check: (page) => {
      const problems: string[] = [];
      const labels = [...page.querySelectorAll(".tab-strip .tab-group-label")].map(
        (label) => label.getAttribute("aria-label") ?? "",
      );
      problems.push(
        ...compare(
          labels,
          ["Billing tab group, 2 tabs, expanded", "Infra tab group, 2 tabs, expanded"],
          "the row of tabs",
        ),
      );
      const lines = page.querySelectorAll(".tab-strip .tab-group-line").length;
      if (lines !== 6) problems.push(`the row draws ${lines} group lines, not 6`);
      return problems;
    },
  },
  {
    name: "tab-groups-folded",
    window: {
      ...GROUPED,
      groups: [
        { tabs: [0, 1], name: "Billing", color: "teal", icon: "bug" },
        { tabs: [2], color: "yellow" },
        { tabs: [3, 4], name: "Infra", color: "blue", icon: "server", collapsed: true },
      ],
    },
    check: (page) => {
      const problems: string[] = [];
      const folded = page.querySelectorAll(".tab-strip .tab[data-folded]").length;
      if (folded !== 2) problems.push(`${folded} tabs are folded away, not 2`);
      const infra = page.querySelector('.tab-strip .tab-group-label[aria-expanded="false"]');
      if (infra?.querySelector(".tab-group-number")?.textContent !== "2") {
        problems.push("the folded group does not say it holds 2 tabs");
      }
      if (!page.querySelector(".tab-strip .tab-group-label[data-empty]")) {
        problems.push("the group with neither a name nor a picture is not drawn as a mark");
      }
      // A label's line is where its tabs' lines are, however tall the label, and runs
      // on into the first tab's.
      for (const label of page.querySelectorAll(
        ".tab-strip .tab-group-label:not([data-collapsed])",
      )) {
        const own = label.querySelector(".tab-group-line")?.getBoundingClientRect();
        const next = label.nextElementSibling?.querySelector(".tab-group-line");
        const first = next?.getBoundingClientRect();
        if (!own || !first) continue;
        const name = label.getAttribute("aria-label") ?? "";
        if (Math.abs(own.top - first.top) > 0.5 || Math.abs(own.bottom - first.bottom) > 0.5) {
          problems.push(`the line under "${name}" is not level with its tabs' line`);
        }
        if (Math.abs(own.right - first.left) > 0.5) {
          problems.push(`the line under "${name}" does not meet its first tab's line`);
        }
      }
      return problems;
    },
  },
  {
    name: "tab-groups-left",
    window: {
      ...GROUPED,
      tabPlacement: "left",
      groups: [
        { tabs: [0, 1], name: "Billing", color: "teal", icon: "bug" },
        { tabs: [3, 4], name: "Infra", color: "blue", icon: "server", collapsed: true },
      ],
    },
    check: (page) => {
      const shown = page.querySelectorAll(".tab-column .tab:not([data-folded])").length;
      return shown === 3 ? [] : [`the column shows ${shown} tabs, not 3`];
    },
  },
  {
    name: "tab-menu",
    window: GROUPED,
    prepare: async () => {
      await openTabMenu("t3");
    },
    check: (page) => {
      const menu = page.querySelector(".tab-menu .menu");
      if (!menu) return ["the tab's menu is not open"];
      return compare(
        rowsOf(menu),
        [
          "Rename tab…",
          "Color",
          "Add tab to new group",
          "Add to group",
          "Billing",
          "Infra",
          "New tab",
          "Move tab left",
          "Move tab right",
          "Move into another tab…",
          "Mute tab",
          "Lock session",
          "Close tabs to the left",
          "Close tabs to the right",
          "Close tab",
        ],
        "the tab's menu",
      );
    },
  },
  {
    name: "tab-menu-move-into",
    window: GROUPED,
    prepare: async () => {
      const menu = await openTabMenu("t1");
      menu.querySelector<HTMLButtonElement>('[data-key="move-into"]')?.click();
      await until("the second step", () => document.querySelector('.tab-menu [data-key="back"]'));
    },
    check: (page) => {
      const menu = page.querySelector(".tab-menu .menu");
      if (!menu) return ["the tab's menu is not open"];
      const problems = compare(
        rowsOf(menu),
        ["Move Fix tax rounding into…", "cargo test", "TODO.md", "terraform apply", "Processes"],
        "the second step",
      );
      if (!menu.querySelector(".menu-note"))
        problems.push("the second step does not explain itself");
      return problems;
    },
  },
  {
    name: "group-editor",
    window: GROUPED,
    prepare: async () => {
      const label = await until("the group's label", () =>
        document.querySelector('.tab-strip .tab-group-label[data-group-color="teal"]'),
      );
      rightClick(label);
      await until("the group's editor", () => document.querySelector(".group-editor"));
    },
    check: (page) => {
      const problems: string[] = [];
      const editor = page.querySelector(".group-editor");
      if (!editor) return ["the group's editor is not open"];
      const name = editor.querySelector<HTMLInputElement>(".group-editor-name")?.value;
      if (name !== "Billing") problems.push(`the editor's name says ${JSON.stringify(name)}`);
      const checked = editor.querySelector('.group-editor-color[aria-checked="true"]');
      if (checked?.getAttribute("aria-label") !== "Teal")
        problems.push("teal is not the colour chosen");
      const kept = editor.querySelector('[aria-pressed="true"] .menu-label')?.textContent;
      if (kept !== "Keep group") problems.push("the group is not shown as kept");
      return problems;
    },
  },
  {
    name: "app-menu-groups",
    window: {
      ...GROUPED,
      groups: [],
      shelf: [
        {
          group: { id: "g90", name: "Release 0.15", color: "pink", icon: "rocket", kept: true },
          tabs: [
            {
              id: "t90",
              title: "changelog",
              root: {
                type: "pane",
                id: "p90",
                title: null,
                content: {
                  type: "terminal",
                  backend: "local",
                  session: null,
                  profile: null,
                  launcher: null,
                  cwd: null,
                  env: [],
                },
              },
              zoomed: null,
              last_focus: null,
              group: "g90",
            },
          ],
        },
      ],
    },
    prepare: async (app) => {
      await shelfArrived(app);
      const button = await until("the application's menu button", () =>
        document.querySelector<HTMLButtonElement>(".app-menu-button"),
      );
      button.click();
      const item = await until("the item that lists the kept groups", () =>
        document.querySelector<HTMLButtonElement>('.app-menu-drop [data-key="groups"]'),
      );
      item.focus();
      item.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
      await until("its menu opening with the keyboard in it", () =>
        document.activeElement?.closest(".menu-sub-place") ? true : null,
      );
    },
    check: (page) => {
      const rows = [...page.querySelectorAll(".menu-sub-place .menu-item .menu-label")].map(
        (label) => label.textContent ?? "",
      );
      return compare(rows, ["Release 0.15"], "the menu of kept groups");
    },
  },
  {
    name: "tabs-overflow",
    window: CROWDED,
    prepare: async () => {
      await crowdedSettled();
    },
    check: (page) => {
      const problems: string[] = [];
      const list = page.querySelector<HTMLElement>(".tab-strip > .tab-list");
      if (!list) return ["there is no row of tabs"];
      const tabs = [...list.querySelectorAll<HTMLElement>(":scope > .tab")];
      if (tabs.length !== MANY) problems.push(`the row holds ${tabs.length} tabs, not ${MANY}`);
      // Tabs keep what they are called: the row scrolls before it squeezes them.
      const narrow = tabs.filter((tab) => tab.getBoundingClientRect().width < 71).length;
      if (narrow > 0) problems.push(`${narrow} tabs are squeezed narrower than they may be`);
      const box = list.getBoundingClientRect();
      const shown = page.querySelector<HTMLElement>('.tab-strip .tab[data-active="true"]');
      const own = shown?.getBoundingClientRect();
      if (!own || own.left < box.left || own.right > box.right + 1) {
        problems.push("the tab on show is not in sight");
      }
      // Only the end that hides tabs fades.
      if (fadeOf(list, "start") !== "28px" || fadeOf(list, "end") !== "0px") {
        problems.push(
          `the row fades ${fadeOf(list, "start")} at its start and ${fadeOf(list, "end")} at its end, not 28px and 0px`,
        );
      }
      if (getComputedStyle(list).maskImage === "none") problems.push("the row has no fade drawn");
      const button = page.querySelector(".tab-strip .tab-all-button");
      if (button?.textContent?.trim() !== String(MANY)) {
        problems.push(
          `the button that lists every tab says "${button?.textContent ?? ""}", not ${MANY}`,
        );
      }
      if (page.querySelector(".tab-all-list"))
        problems.push("the list of every tab is open unasked");
      return problems;
    },
  },
  {
    // A tab's name is written without the row being drawn again, and a full row keeps
    // its size: the mark is to follow its tab all the same.
    name: "tabs-rename-overflow",
    window: CROWDED,
    prepare: async (app) => {
      await crowdedSettled();
      const tab = await tabOf(`t${MANY}`);
      const before = tab.getBoundingClientRect().width;
      app.commands.run("lume.tab.rename", {
        tab: `t${MANY}`,
        title: "release checklist for the billing service",
      });
      await until("the tab growing with its new name", () =>
        tab.getBoundingClientRect().width > before + 8 ? true : null,
      );
      await crowdedSettled();
      // The row is measured when the frame is drawn.
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    },
    check: (page) => {
      const tab = page.querySelector<HTMLElement>('.tab-strip .tab[data-active="true"]');
      const mark = page.querySelector<HTMLElement>(".tab-strip .tab-mark");
      if (!tab || !mark) return ["there is no tab on show, or no mark"];
      const own = tab.getBoundingClientRect();
      const drawn = mark.getBoundingClientRect();
      const problems: string[] = [];
      if (Math.abs(drawn.left - own.left) > 2 || Math.abs(drawn.right - own.right) > 2) {
        problems.push(
          `the mark spans ${Math.round(drawn.left)}–${Math.round(drawn.right)}, its tab ${Math.round(own.left)}–${Math.round(own.right)}`,
        );
      }
      const list = tab.parentElement?.getBoundingClientRect();
      if (list && own.right > list.right + 1) problems.push("the renamed tab is not in sight");
      return problems;
    },
  },
  {
    name: "tabs-all-list",
    window: CROWDED,
    prepare: async () => {
      await crowdedSettled();
      const button = await until("the button that lists every tab", () =>
        document.querySelector<HTMLButtonElement>(".tab-strip .tab-all-button"),
      );
      button.click();
      const field = await until("the list's field taking the keyboard", () =>
        document.activeElement instanceof HTMLInputElement &&
        document.activeElement.closest(".tab-all-list")
          ? document.activeElement
          : null,
      );
      // Typed as a person types, so that the page hears of it.
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      set?.call(field, "t");
      field.dispatchEvent(new Event("input", { bubbles: true }));
      const wanted = MANY_TITLES.filter((title) => title.toLowerCase().includes("t")).length;
      await until("the list narrowing to what was typed", () =>
        document.querySelectorAll(".tab-all-list .tab-all-row").length === wanted ? true : null,
      );
    },
    check: (page) => {
      const problems: string[] = [];
      const rows = [...page.querySelectorAll(".tab-all-list .tab-all-row")];
      const wanted = MANY_TITLES.filter((title) => title.toLowerCase().includes("t"));
      problems.push(
        ...compare(
          rows.map((row) => row.querySelector(".tab-all-title")?.textContent ?? ""),
          wanted,
          "the list of every tab",
        ),
      );
      const current = rows.filter((row) => row.querySelector('small[data-kind="current"]'));
      if (
        current.length !== 1 ||
        current[0]?.querySelector(".tab-all-title")?.textContent !== "scratch"
      ) {
        problems.push("the list does not say which tab is on show");
      }
      const hidden = rows.filter((row) => row.querySelector("small:not([data-kind])")).length;
      if (hidden === 0) problems.push("the list says of no tab that it is out of view");
      // The first tabs are the ones scrolled away, with the last one on show.
      if (!rows[0]?.querySelector("small:not([data-kind])")) {
        problems.push("the first tab is not said to be out of view");
      }
      if (rows.filter((row) => row.getAttribute("aria-selected") === "true").length !== 1) {
        problems.push("the list does not mark one row for the keyboard");
      }
      if (page.querySelector(".tab-all-button")?.getAttribute("aria-expanded") !== "true") {
        problems.push("the button does not say its list is open");
      }
      return problems;
    },
  },
];
