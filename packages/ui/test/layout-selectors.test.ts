import { describe, expect, it } from "vitest";

import {
  hasTabListPanel,
  isFloating,
  isFocused,
  isPaneVisible,
  selectActivePanes,
  selectActiveTab,
  selectDock,
  selectFloatRect,
  selectFloating,
  selectFocus,
  selectPane,
  selectPanelArea,
  selectPanels,
  selectRatios,
  selectSplit,
  selectTab,
  selectTabOf,
  selectTabOrder,
  selectTabPanes,
  selectTitlePane,
  selectVisiblePanes,
  selectWindowOf,
  selectWindowPanes,
  selectZoomed,
  TAB_LIST_PANEL,
  tabTitle,
  UNTITLED_TAB,
} from "../src/layout/selectors.ts";
import { createLayoutStore, type LayoutStore } from "../src/layout/store.ts";
import type { Workspace } from "../src/layout/types.ts";
import { oneTab, pane, split, viewOf } from "./layout-support.ts";

const RECT = { x: 100_000, y: 100_000, w: 500_000, h: 500_000 };

/** Window w1: tab t1 (p1 beside p2 over p3, on show), tab t2 (p4), and p5 floating. */
function workspace(): Workspace {
  const base = oneTab(
    split(
      "s1",
      "row",
      [500_000, 500_000],
      [pane("p1"), split("s2", "column", [400_000, 600_000], [pane("p2", "build"), pane("p3")])],
    ),
    "p2",
  );
  const floating = pane("p5");
  if (floating.type !== "pane") throw new Error("not a pane");
  return {
    ...base,
    windows: base.windows.map((window) => ({
      ...window,
      centre: {
        active: "t1",
        tabs: [
          ...window.centre.tabs,
          { id: "t2", title: null, root: pane("p4"), zoomed: null, last_focus: "p4" },
        ],
      },
      floating: [
        { pane: { id: "p5", title: null, content: floating.content }, rect: RECT, minimised: null },
      ],
    })),
  };
}

function started(): LayoutStore {
  const store = createLayoutStore();
  store.getState().reset(viewOf(workspace()));
  return store;
}

describe("questions about a window", () => {
  const state = started().getState();

  it("answers for the page's own window when none is named", () => {
    expect(selectTabOrder(state)).toEqual(["t1", "t2"]);
    expect(selectActiveTab(state)).toBe("t1");
    expect(selectFocus(state)).toBe("p2");
    expect(selectFloating(state)).toEqual(["p5"]);
    expect(selectTabOrder(state, "w1")).toBe(selectTabOrder(state));
  });

  it("has the same empty answers for a window there is not", () => {
    expect(selectTabOrder(state, "w9")).toBe(selectFloating(state, "w9"));
    expect(selectActiveTab(state, "w9")).toBeNull();
    expect(selectFocus(state, "w9")).toBeNull();
    expect(selectVisiblePanes(state, "w9")).toEqual([]);
  });

  it("lists panes in the order they are laid out", () => {
    expect(selectTabPanes(state, "t1")).toEqual(["p1", "p2", "p3"]);
    expect(selectActivePanes(state)).toEqual(["p1", "p2", "p3"]);
    expect(selectVisiblePanes(state)).toEqual(["p1", "p2", "p3", "p5"]);
    expect(selectWindowPanes(state)).toEqual(["p1", "p2", "p3", "p4", "p5"]);
    expect(selectTabPanes(state, "t9")).toEqual([]);
  });
});

describe("questions about one thing", () => {
  const state = started().getState();

  it("finds entries by id", () => {
    expect(selectTab(state, "t2")?.root).toEqual({ type: "pane", id: "p4" });
    expect(selectPane(state, "p2")?.title).toBe("build");
    expect(selectSplit(state, "s2")?.axis).toBe("column");
    expect(selectRatios(state, "s2")).toEqual([400_000, 600_000]);
    expect(selectRatios(state, "s9")).toBeUndefined();
    expect(selectZoomed(state, "t1")).toBeNull();
  });

  it("says where a pane is", () => {
    expect(selectTabOf(state, "p3")).toBe("t1");
    expect(selectTabOf(state, "p5")).toBeUndefined();
    expect(selectWindowOf(state, "p4")).toBe("w1");
    expect(selectWindowOf(state, "p5")).toBe("w1");
    expect(selectWindowOf(state, "p9")).toBeUndefined();
    expect(isFloating(state, "p5")).toBe(true);
    expect(isFloating(state, "p1")).toBe(false);
    expect(selectFloatRect(state, "p5")).toEqual(RECT);
    expect(isFocused(state, "p2")).toBe(true);
    expect(isFocused(state, "p1")).toBe(false);
  });
});

describe("whether a pane can be seen", () => {
  it("can be when it floats or is in the tab on show", () => {
    const state = started().getState();
    expect(["p1", "p2", "p3", "p5"].every((id) => isPaneVisible(state, id))).toBe(true);
    expect(isPaneVisible(state, "p4")).toBe(false);
    expect(isPaneVisible(state, "p9")).toBe(false);
  });

  it("cannot be behind a zoomed pane", () => {
    const store = started();
    const root = workspace().windows[0]!.centre.tabs[0]!.root;
    store.getState().apply(1, 1, [{ type: "tab_tree", tab: "t1", root, zoomed: "p3" }]);
    const state = store.getState();
    expect(isPaneVisible(state, "p3")).toBe(true);
    expect(isPaneVisible(state, "p1")).toBe(false);
    expect(isPaneVisible(state, "p5")).toBe(true);
    expect(selectVisiblePanes(state)).toEqual(["p3", "p5"]);
    expect(selectZoomed(state, "t1")).toBe("p3");
  });

  it("follows the tab on show", () => {
    const store = started();
    store
      .getState()
      .apply(1, 1, [{ type: "tabs", window: "w1", order: ["t1", "t2"], active: "t2" }]);
    const state = store.getState();
    expect(isPaneVisible(state, "p4")).toBe(true);
    expect(isPaneVisible(state, "p1")).toBe(false);
    expect(selectActivePanes(state)).toEqual(["p4"]);
  });
});

describe("what a tab is called", () => {
  const titles = new Map([
    ["p1", "vim"],
    ["p3", "htop"],
    ["p4", "zsh"],
  ]);

  it("is the name of the pane that last had the keyboard", () => {
    const store = started();
    // p2 was named by the user, which goes before what its program says.
    expect(selectTitlePane(store.getState(), "t1")).toBe("p2");
    expect(tabTitle(store.getState(), "t1", titles)).toBe("build");
    store.getState().apply(1, 1, [{ type: "focus", window: "w1", pane: "p3" }]);
    expect(tabTitle(store.getState(), "t1", titles)).toBe("htop");
    expect(tabTitle(store.getState(), "t2", titles)).toBe("zsh");
  });

  it("is the first pane's when the last focus is not known or has gone", () => {
    const store = createLayoutStore();
    store.getState().reset(viewOf(oneTab(split("s1", "row", [1, 2], [pane("p1"), pane("p2")]))));
    expect(selectTitlePane(store.getState(), "t1")).toBe("p1");
    expect(tabTitle(store.getState(), "t1", titles)).toBe("vim");

    const moved = started();
    moved.getState().apply(1, 1, [
      { type: "tab_tree", tab: "t1", root: pane("p1"), zoomed: null },
      { type: "focus", window: "w1", pane: "p1" },
    ]);
    expect(tabTitle(moved.getState(), "t1", titles)).toBe("vim");
  });

  it("is the user's name for the tab before anything else", () => {
    const store = started();
    store.getState().apply(1, 1, [{ type: "tab_title", tab: "t1", title: "server" }]);
    expect(tabTitle(store.getState(), "t1", titles)).toBe("server");
  });

  it("is a plain word when nothing names it", () => {
    const state = started().getState();
    expect(tabTitle(state, "t2", new Map())).toBe(UNTITLED_TAB);
    expect(tabTitle(state, "t9", titles)).toBe(UNTITLED_TAB);
    expect(UNTITLED_TAB).toBe("Terminal");
  });
});

describe("the panels at a window's sides", () => {
  /** One tab, with the list of tabs docked at `side` and a plugin's view at the bottom. */
  function docked(side: "left" | "right" | null): LayoutStore {
    const base = oneTab(pane("p1"), "p1");
    const window = base.windows[0]!;
    const empty = { panels: [], size: 240, collapsed: false };
    const tabs = { id: "lume.tabs", content: { type: "tab_list" as const } };
    const view = {
      id: "acme.files",
      content: { type: "plugin_view" as const, plugin: "acme", view: "files" },
    };
    const store = createLayoutStore();
    store.getState().reset(
      viewOf({
        ...base,
        windows: [
          {
            ...window,
            dock: {
              top: empty,
              left: side === "left" ? { ...empty, panels: [tabs], size: 300 } : empty,
              right: side === "right" ? { ...empty, panels: [tabs] } : empty,
              bottom: { ...empty, panels: [view] },
            },
          },
        ],
      }),
    );
    return store;
  }

  it("finds the side a panel is docked at", () => {
    expect(selectPanelArea(docked("left").getState(), TAB_LIST_PANEL)).toBe("left");
    expect(selectPanelArea(docked("right").getState(), TAB_LIST_PANEL)).toBe("right");
    expect(selectPanelArea(docked(null).getState(), TAB_LIST_PANEL)).toBeNull();
    expect(selectPanelArea(docked(null).getState(), "acme.files")).toBe("bottom");
    expect(selectPanelArea(docked("left").getState(), "acme.files", "w9")).toBeNull();
  });

  it("says whether the tabs are listed at a side, which takes them out of the top bar", () => {
    expect(hasTabListPanel(docked("left").getState())).toBe(true);
    expect(hasTabListPanel(docked(null).getState())).toBe(false);
    expect(hasTabListPanel(createLayoutStore().getState())).toBe(false);

    // A side that is folded away shows no list, so the top bar lists the tabs again.
    const store = docked("left");
    const left = selectDock(store.getState(), "left")!;
    store
      .getState()
      .apply(1, 1, [
        { type: "dock", window: "w1", area: "left", dock: { ...left, collapsed: true } },
      ]);
    expect(selectPanelArea(store.getState(), TAB_LIST_PANEL)).toBe("left");
    expect(hasTabListPanel(store.getState())).toBe(false);
  });

  it("gives a side's area and its panels, the same until that side changes", () => {
    const store = docked("left");
    const before = store.getState();
    expect(selectDock(before, "left")?.size).toBe(300);
    expect(selectPanels(before, "left").map((panel) => panel.id)).toEqual(["lume.tabs"]);
    expect(selectPanels(before, "right")).toEqual([]);
    expect(selectPanels(createLayoutStore().getState(), "left")).toEqual([]);

    store.getState().apply(1, 1, [{ type: "dock_size", window: "w1", area: "bottom", size: 180 }]);
    const after = store.getState();
    expect(selectDock(after, "bottom")?.size).toBe(180);
    expect(selectDock(after, "left")).toBe(selectDock(before, "left"));
    expect(selectPanels(after, "left")).toBe(selectPanels(before, "left"));
  });
});
