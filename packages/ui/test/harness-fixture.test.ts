import { DEFAULT_TOKENS, LIGHT_TOKENS } from "@lume/tokens";
import { describe, expect, it } from "vitest";

import type { ConfigView, LayoutView, Notice } from "../src/core/types.ts";
import { FakeCore, FIXTURE_SESSIONS, FIXTURE_TABS } from "../src/harness/fake-core.ts";
import type { Node, Workspace } from "../src/layout/types.ts";
import { HEADER_LEN } from "../src/transport/frames.ts";

/** A core with what it tells its page kept. */
async function opened(core: FakeCore): Promise<Notice[]> {
  const notices: Notice[] = [];
  await core.open((frames) => {
    const text = new TextDecoder().decode(frames.subarray(HEADER_LEN));
    const message = JSON.parse(text) as { notify?: Notice };
    if (message.notify) notices.push(message.notify);
  });
  return notices;
}

async function workspace(core: FakeCore): Promise<Workspace> {
  return (await core.command<LayoutView>("layout_view")).workspace;
}

function terminal(id: string, session: string): Node {
  return {
    type: "pane",
    id,
    title: null,
    content: {
      type: "terminal",
      backend: "local",
      session,
      profile: null,
      launcher: null,
      cwd: null,
      env: [],
    },
  };
}

function panesOf(node: Node): string[] {
  return node.type === "pane" ? [node.id] : node.children.flatMap(panesOf);
}

describe("the stand-in core's window when nothing is asked of it", () => {
  it("is one window with two panes side by side in one tab and one pane in another", async () => {
    const dock = { panels: [], size: 240, collapsed: false };
    expect(await workspace(new FakeCore())).toEqual({
      id: "00000000000000aa",
      name: "fixture",
      windows: [
        {
          id: "w1",
          bounds: null,
          dock: { top: dock, left: dock, right: dock, bottom: dock },
          centre: {
            tabs: [
              {
                id: "t1",
                title: null,
                root: {
                  type: "split",
                  id: "s1",
                  axis: "row",
                  ratios: [500_000, 500_000],
                  children: [
                    terminal("p1", "0000000000000001"),
                    terminal("p2", "0000000000000002"),
                  ],
                },
                zoomed: null,
                last_focus: "p1",
              },
              {
                id: "t2",
                title: null,
                root: terminal("p3", "0000000000000003"),
                zoomed: null,
                last_focus: "p3",
              },
            ],
            active: "t1",
          },
          floating: [],
          focus: "p1",
          history: ["p1"],
        },
      ],
      focused_window: "w1",
      scratch: null,
      next_id: 10,
    });
  });

  it("names its panes' sessions as the suites that flood them expect", () => {
    const core = new FakeCore();
    expect(core.panes).toEqual(["p1", "p2", "p3"]);
    for (const pane of core.panes) expect(core.sessionOf(pane)).toBe(FIXTURE_SESSIONS[pane]);
    expect(core.tabs).toEqual([FIXTURE_TABS.first, FIXTURE_TABS.second]);
  });

  it("has the dark look, no commands and no keys", async () => {
    const core = new FakeCore();
    const view = await core.command<ConfigView>("config_view");
    expect(view.tokens).toBe(DEFAULT_TOKENS);
    expect(view.theme).toEqual({ id: "lume-dark", scheme: "dark" });
    expect(view.title_bar).toBe("system");
    expect(view.config.terminal["scrollback-lines"]).toBe(1000);
    expect(view.keymap.bindings).toEqual([]);
    expect(await core.command("commands_list")).toEqual([]);
  });

  it("shows a tab and gives its first pane the keyboard", async () => {
    const core = new FakeCore();
    const notices = await opened(core);
    core.activate("t2");
    expect(notices).toEqual([
      {
        type: "layout_changed",
        epoch: 1,
        revision: 2,
        changes: [
          { type: "tabs", window: "w1", order: ["t1", "t2"], active: "t2" },
          { type: "focus", window: "w1", pane: "p3" },
        ],
      },
    ]);
    const shown = (await workspace(core)).windows[0];
    expect(shown?.centre.active).toBe("t2");
    expect(shown?.focus).toBe("p3");
  });
});

describe("the stand-in core's window as a test asks for it", () => {
  it("gives each tab the panes asked for, numbered through", async () => {
    const core = new FakeCore({ tabs: [1, 2, 3, 4] });
    const tabs = (await workspace(core)).windows[0]?.centre.tabs ?? [];
    expect(tabs.map((tab) => panesOf(tab.root))).toEqual([
      ["p1"],
      ["p2", "p3"],
      ["p4", "p5", "p6"],
      ["p7", "p8", "p9", "p10"],
    ]);
    expect(core.panesOf("t4")).toEqual(["p7", "p8", "p9", "p10"]);
    // Four panes are two columns of two.
    const four = tabs[3]?.root;
    expect(four?.type === "split" && four.axis).toBe("row");
    expect(four?.type === "split" && four.children.map((child) => child.type)).toEqual([
      "split",
      "split",
    ]);
    // Past the ninth pane a session is still sixteen hexadecimal digits.
    expect(core.sessionOf("p10")).toBe("000000000000000a");
  });

  it("refuses a tab of no panes or of too many", () => {
    expect(() => new FakeCore({ tabs: [0] })).toThrow();
    expect(() => new FakeCore({ tabs: [5] })).toThrow();
  });

  it("lists the tabs at a side, docks a panel, floats a pane and puts one away", async () => {
    const core = new FakeCore({
      tabs: [2],
      tabPlacement: "left",
      dockPanel: "right",
      floating: true,
      minimised: true,
    });
    const shown = (await workspace(core)).windows[0];
    expect(shown?.dock.left.panels).toEqual([{ id: "lume.tabs", content: { type: "tab_list" } }]);
    expect(shown?.dock.right.panels.map((panel) => panel.content.type)).toEqual(["plugin_view"]);
    expect(shown?.floating.map((float) => [float.pane.id, float.minimised])).toEqual([
      ["p4", "tab"],
      ["p3", null],
    ]);
    const view = await core.command<ConfigView>("config_view");
    expect(view.config.layout["tab-placement"]).toBe("left");
  });

  it("tells the page of a split resized, a pane focused and a dock area resized", async () => {
    const core = new FakeCore({ tabs: [2] });
    const notices = await opened(core);
    await core.command("layout_apply", {
      intent: { type: "resize", split: "s1", ratios: [300_000, 700_000] },
    });
    await core.command("layout_apply", {
      intent: { type: "focus", target: { type: "pane", pane: "p2" } },
    });
    // The pane that has the keyboard already is not news.
    await core.command("layout_apply", {
      intent: { type: "focus", target: { type: "pane", pane: "p2" } },
    });
    await core.command("layout_apply", {
      intent: { type: "panel_resize", window: "w1", area: "left", size: 300 },
    });
    expect(notices.map((notice) => notice.type === "layout_changed" && notice.changes)).toEqual([
      [{ type: "ratios", split: "s1", ratios: [300_000, 700_000] }],
      [{ type: "focus", window: "w1", pane: "p2" }],
      [{ type: "dock_size", window: "w1", area: "left", size: 300 }],
    ]);
    const shown = (await workspace(core)).windows[0];
    const root = shown?.centre.tabs[0]?.root;
    expect(root?.type === "split" && root.ratios).toEqual([300_000, 700_000]);
    expect(shown?.focus).toBe("p2");
    expect(shown?.dock.left.size).toBe(300);
  });

  it("changes to the light look and says so once", async () => {
    const core = new FakeCore({ tier: "webgl2", titleBar: "custom", scrollbackLines: 10_000 });
    const notices = await opened(core);
    core.setScheme("light");
    core.setScheme("light");
    expect(notices).toHaveLength(1);
    const notice = notices[0];
    if (notice?.type !== "config_changed") throw new Error("not a change of configuration");
    expect(notice.revision).toBe(2);
    expect(notice.view.tokens).toEqual(LIGHT_TOKENS);
    expect(notice.view.theme).toEqual({ id: "lume-light", scheme: "light" });
    expect(notice.view.config.renderer.tier).toBe("webgl2");
    expect(notice.view.title_bar).toBe("custom");
    expect(notice.view.config.terminal["scrollback-lines"]).toBe(10_000);
  });

  it("answers what the settings window asks and still says what it cannot answer", async () => {
    const core = new FakeCore();
    const schema = await core.command<Record<string, unknown>>("config_schema");
    expect(schema["x-lume-pages"]).toBeInstanceOf(Array);
    expect(await core.command<unknown[]>("themes_list")).toHaveLength(2);
    expect((await core.command<unknown[]>("shells_list")).length).toBeGreaterThan(0);
    expect(await core.command<string[]>("font_families")).toContain("JetBrains Mono");
    expect((await core.command<ArrayBuffer>("background_image")).byteLength).toBe(0);
    await expect(core.command("config_unset", { path: "font.size" })).resolves.toBeNull();
    expect(core.unanswered).toEqual([]);
    await expect(core.command("no_such_command")).rejects.toMatchObject({ code: "unsupported" });
    expect(core.unanswered).toEqual(["no_such_command"]);
  });
});
