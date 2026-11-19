import { describe, expect, it, vi } from "vitest";

import type { ConfigView, MenuNodeData, MotionDef, UiContributions } from "../src/core/types.ts";
import { tabStyle, tabStyleName } from "../src/motion/tab.ts";
import { pluginStatusItems } from "../src/plugins/status.ts";
import { createPluginsStore, holds, iconNamed } from "../src/plugins/store.ts";
import { VisibleItems } from "../src/plugins/visible.ts";
import { joinPath, splitPath } from "../src/settings/model.ts";
import { flatten } from "../src/shell/ListPanel.tsx";
import type { MenuItem } from "../src/shell/Menu.tsx";
import { menuRows, nodesOf, type MenuParts } from "../src/shell/menus.ts";
import { cursorFxSettings } from "../src/terminal/cursor-fx.ts";

const EMPTY: UiContributions = {
  revision: 1,
  plugins: [],
  menus: {},
  status: [],
  panels: [],
  images: {},
  motions: [],
  cursor_effects: [],
  values: [],
  rows: {},
  contexts: {},
};

function parts(contexts: ReadonlySet<string> = new Set()): MenuParts {
  return {
    contexts,
    builtin: (key) => [{ key, label: key, choose: () => undefined }],
    plugin: (node) =>
      node.command === undefined
        ? undefined
        : { key: node.key, label: node.title ?? node.command, choose: () => undefined },
  };
}

function labels(rows: readonly MenuItem[]): string[] {
  return rows.map(
    (row) => `${row.separated ? "| " : ""}${row.kind === "heading" ? "# " : ""}${row.label}`,
  );
}

describe("menus drawn from what the core placed", () => {
  it("sets sections apart, heads a plugin's section with its title, and runs loose items together", () => {
    const nodes: MenuNodeData[] = [
      { kind: "item", key: "acme:first", plugin: "acme", title: "First", command: "acme.first" },
      { kind: "item", key: "acme:second", plugin: "acme", title: "Second", command: "acme.second" },
      { kind: "section", key: "lume:zoom", children: [{ kind: "item", key: "lume:zoom-in" }] },
      {
        kind: "section",
        key: "acme:meters",
        plugin: "acme",
        title: "Meters",
        children: [
          {
            kind: "item",
            key: "acme:refresh",
            plugin: "acme",
            title: "Refresh",
            command: "acme.refresh",
          },
          {
            kind: "item",
            key: "acme:busy",
            plugin: "acme",
            title: "While busy",
            command: "acme.busy",
            when: ["plugin:acme.busy"],
          },
        ],
      },
      { kind: "section", key: "lume:empty", children: [] },
      { kind: "item", key: "acme:last", plugin: "acme", title: "Last", command: "acme.last" },
    ];
    expect(labels(menuRows(nodes, parts()))).toEqual([
      "First",
      "Second",
      "| zoom-in",
      "| # Meters",
      "Refresh",
      "| Last",
    ]);
    expect(labels(menuRows(nodes, parts(new Set(["plugin:acme.busy"]))))).toContain("While busy");
  });

  it("opens a plugin's submenu with its items, and leaves out one that has none", () => {
    const rows = menuRows(
      [
        {
          kind: "submenu",
          key: "acme:tools",
          plugin: "acme",
          title: "Tools",
          children: [
            { kind: "item", key: "acme:go", plugin: "acme", title: "Go", command: "acme.go" },
          ],
        },
        { kind: "submenu", key: "acme:none", plugin: "acme", title: "None", children: [] },
      ],
      parts(),
    );
    expect(rows.map((row) => row.label)).toEqual(["Tools"]);
    expect(rows[0]?.items?.map((row) => row.label)).toEqual(["Go"]);
  });

  it("draws a menu from the application's own nodes while the core has sent none", () => {
    const keys = nodesOf(undefined, "menu.tab").map((node) => node.key);
    expect(keys).toEqual(["lume:name", "lume:group", "lume:position", "lume:closing"]);
    expect(nodesOf({ "menu.tab": [] }, "menu.tab")).toEqual([]);
  });
});

describe("the plugins' status items", () => {
  const shown: UiContributions = {
    ...EMPTY,
    plugins: [{ id: "acme", name: "Acme", version: "1.0.0", ready: true, problem: null }],
    status: [
      {
        id: "acme.cpu",
        plugin: "acme",
        slot: "topbar.right",
        kind: "meter",
        order: 0,
        label: "CPU",
      },
      {
        id: "acme.gpu",
        plugin: "acme",
        slot: "topbar.right",
        kind: "meter",
        order: 1,
        label: "GPU",
        shown_when: { key: "gpu", default: true },
      },
      {
        id: "acme.badge",
        plugin: "acme",
        slot: "pane.header",
        kind: "badge",
        order: 2,
        command: "acme.go",
      },
    ],
    images: { "acme:chip": { url: "data:image/png;base64,AA==", tint: false } },
    values: [
      {
        plugin: "acme",
        item: "acme.cpu",
        pane: null,
        value: { value: 0.5, text: "50%", icon: "chip" },
      },
      { plugin: "acme", item: "acme.badge", pane: "p1", value: { text: "2" } },
      { plugin: "acme", item: "acme.badge", pane: "p2", value: { text: "" } },
    ],
  };

  it("are drawn from their manifests and what their code shows", () => {
    const store = createPluginsStore();
    store.getState().set(shown);
    const items = pluginStatusItems(store.getState());
    expect(items.map((item) => item.id)).toEqual(["acme.cpu", "acme.gpu", "acme.badge@p1"]);
    const cpu = items[0];
    expect(cpu?.kind === "meter" && cpu.value).toBe(0.5);
    expect(cpu?.picture).toEqual({ image: "acme:chip" });
    const badge = items[2];
    expect(badge?.pane).toBe("p1");
    expect(badge?.args).toEqual({ pane: "p1" });
  });

  it("follow the switch of the plugin's settings they are shown under", () => {
    const store = createPluginsStore();
    store.getState().set(shown);
    const view = { config: { plugins: { acme: { gpu: false } } } } as unknown as ConfigView;
    expect(pluginStatusItems(store.getState(), view).map((item) => item.id)).not.toContain(
      "acme.gpu",
    );
  });

  it("are hidden by their code, and show a plugin held up by a problem", () => {
    const store = createPluginsStore();
    store.getState().set(shown);
    store.getState().follow({
      type: "plugin_status",
      plugin: "acme",
      item: "acme.cpu",
      pane: null,
      value: { hidden: true },
    });
    store
      .getState()
      .follow({ type: "plugin_state", plugin: "acme", problem: "it went over its time" });
    const items = pluginStatusItems(store.getState());
    expect(items.map((item) => item.id)).toEqual(["acme.gpu", "acme.badge@p1"]);
    expect(items[0]?.problem).toBe("it went over its time");
  });

  it("name pictures of the plugin's own before the page's", () => {
    expect(iconNamed("acme", "chip", shown.images)).toEqual({ image: "acme:chip" });
    expect(iconNamed("acme", "cpu", shown.images)).toEqual({ glyph: "cpu" });
    expect(holds(["!plugin:acme.x", "terminal"], new Set(["terminal"]))).toBe(true);
  });
});

describe("what the window has on screen", () => {
  it("is told once a burst of changes settles, and as nothing while the window is out of sight", () => {
    vi.useFakeTimers();
    const told: string[][] = [];
    const visible = new VisibleItems((items) => told.push([...items]));
    const release = visible.hold("acme.cpu");
    visible.hold("acme.gpu");
    vi.runAllTimers();
    expect(told).toEqual([["acme.cpu", "acme.gpu"]]);
    release();
    visible.setShown(false);
    vi.runAllTimers();
    expect(told.at(-1)).toEqual([]);
    visible.setShown(true);
    vi.runAllTimers();
    expect(told.at(-1)).toEqual(["acme.gpu"]);
    visible.dispose();
    vi.useRealTimers();
  });
});

describe("paths of settings", () => {
  it("quote a key that holds a dot, as the core writes them", () => {
    const path = joinPath(["plugins", "acme.meters", "rate"]);
    expect(path).toBe('plugins."acme.meters".rate');
    expect(splitPath(path)).toEqual(["plugins", "acme.meters", "rate"]);
    expect(splitPath("font.size")).toEqual(["font", "size"]);
    expect(splitPath(joinPath(["a", 'say "hi"', "b"]))).toEqual(["a", 'say "hi"', "b"]);
  });
});

describe("a list panel's rows", () => {
  it("are drawn in order, with those of closed rows left out", () => {
    const rows = [
      { id: "a", label: "A", expanded: true, children: [{ id: "a1", label: "A1" }] },
      { id: "b", label: "B", children: [{ id: "b1", label: "B1" }] },
    ];
    const drawn = flatten(rows, new Map());
    expect(drawn.map((row) => row.row.id)).toEqual(["a", "a1", "b"]);
    expect(drawn[1]?.parent).toBe(0);
    expect(drawn[1]?.depth).toBe(1);
    const opened = flatten(
      rows,
      new Map([
        ["b", true],
        ["a", false],
      ]),
    );
    expect(opened.map((row) => row.row.id)).toEqual(["a", "b", "b1"]);
  });
});

describe("plugins' effects chosen in the settings", () => {
  it("draw a plugin's shader for the trail, and nothing for one that is not there", () => {
    const shown: UiContributions = {
      ...EMPTY,
      cursor_effects: [
        {
          id: "acme/glow",
          plugin: "acme",
          title: "Glow",
          kind: "trail",
          duration: 300,
          reach: 32,
          source: "x",
        },
      ],
    };
    const view = {
      config: { terminal: { "cursor-trail": "acme/glow", "cursor-pulse": "gone/thing" } },
    } as unknown as ConfigView;
    const settings = cursorFxSettings(view, shown);
    expect(settings.trail).toBe("none");
    expect(settings.customTrail?.id).toBe("acme/glow");
    expect(settings.pulse).toBe("none");
    expect(settings.customPulse).toBeUndefined();
  });

  it("move the tab's mark with a plugin's springs, drawn as the window's style of its shape", () => {
    const motion: MotionDef = {
      id: "acme/bouncy",
      plugin: "acme",
      title: "Bouncy",
      slot: "tab.highlight",
      spec: {
        slot: "tab.highlight",
        highlight: {
          shape: "underline",
          lead: [700, 0.9],
          trail: [300, 1],
          slide: "spring",
          enter: { keyframes: [{ opacity: 0 }, { opacity: 1 }], spring: [600, 0.7] },
          renamed: null,
        },
      },
    };
    const style = tabStyle("acme/bouncy", [motion]);
    expect(typeof style === "object" && style.lead).toEqual([700, 0.9]);
    expect(tabStyleName(style)).toBe("stretch");
    expect(tabStyle("gone/thing", [motion])).toBe("glow");
    expect(tabStyle("spring", [motion])).toBe("spring");
  });
});
