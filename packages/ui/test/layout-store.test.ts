import { describe, expect, it } from "vitest";

import {
  isMinimised,
  isPaneVisible,
  selectMinimised,
  selectVisiblePanes,
} from "../src/layout/selectors.ts";
import { createLayoutStore, toWorkspace } from "../src/layout/store.ts";
import type { LayoutChange } from "../src/layout/types.ts";
import {
  followerForm,
  oneTab,
  pane,
  PlainFollower,
  split,
  vector,
  VECTOR_NAMES,
  viewOf,
} from "./layout-support.ts";

describe("a layout store following the recorded sequences", () => {
  for (const name of VECTOR_NAMES) {
    it(`holds what a plain follower holds after every step of ${name}`, () => {
      const recorded = vector(name);
      const store = createLayoutStore();
      store.getState().reset(viewOf(recorded.start));
      const plain = new PlainFollower(recorded.start);
      expect(toWorkspace(store.getState())).toEqual(plain.workspace);

      let revision = 0;
      let applied = 0;
      for (const step of recorded.steps) {
        // A refused intent, and one that changed nothing, are not numbered.
        if (!step.changes || step.changes.length === 0) continue;
        revision += 1;
        expect(store.getState().apply(1, revision, step.changes)).toBe("applied");
        for (const change of step.changes) plain.apply(change);
        expect(toWorkspace(store.getState()), `after step ${applied}`).toEqual(plain.workspace);
        applied += 1;
      }
      expect(applied).toBeGreaterThan(5);
      expect(store.getState().revision).toBe(revision);
      expect(toWorkspace(store.getState())).toEqual(followerForm(recorded.end));
    });
  }

  it("keeps nothing of panes, tabs and splits that are gone", () => {
    for (const name of VECTOR_NAMES) {
      const recorded = vector(name);
      const store = createLayoutStore();
      store.getState().reset(viewOf(recorded.start));
      let revision = 0;
      for (const step of recorded.steps) {
        if (!step.changes || step.changes.length === 0) continue;
        store.getState().apply(1, ++revision, step.changes);
      }
      const state = store.getState();
      const end = recorded.end;
      const panes = end.windows.flatMap((window) => [
        ...window.centre.tabs.flatMap((tab) => idsOf(tab.root, "pane")),
        ...window.floating.map((floating) => floating.pane.id),
      ]);
      const splits = end.windows.flatMap((window) =>
        window.centre.tabs.flatMap((tab) => idsOf(tab.root, "split")),
      );
      const tabs = end.windows.flatMap((window) => window.centre.tabs.map((tab) => tab.id));
      expect(Object.keys(state.panes).sort(), name).toEqual([...panes].sort());
      expect(Object.keys(state.splits).sort(), name).toEqual([...splits].sort());
      expect(Object.keys(state.tabs).sort(), name).toEqual([...tabs].sort());
      expect(Object.keys(state.windows).sort(), name).toEqual(
        end.windows.map((window) => window.id).sort(),
      );
      for (const pane of Object.keys(state.paneTabs)) expect(panes).toContain(pane);
      for (const pane of Object.keys(state.floats)) expect(panes).toContain(pane);
    }
  });
});

function idsOf(node: ReturnType<typeof pane>, kind: "pane" | "split"): string[] {
  if (node.type === "pane") return kind === "pane" ? [node.id] : [];
  const below = node.children.flatMap((child) => idsOf(child, kind));
  return kind === "split" ? [node.id, ...below] : below;
}

/** A row of `p1` and a column of `p2` over `p3`. */
function threePanes() {
  return oneTab(
    split(
      "s1",
      "row",
      [500_000, 500_000],
      [pane("p1"), split("s2", "column", [400_000, 600_000], [pane("p2"), pane("p3")])],
    ),
    "p1",
  );
}

function started() {
  const store = createLayoutStore();
  store.getState().reset(viewOf(threePanes(), "w1", 3, 10));
  return store;
}

const RENAME: LayoutChange = { type: "tab_title", tab: "t1", title: "logs" };

describe("the order of changes", () => {
  it("has nothing to follow before it was given a layout", () => {
    const store = createLayoutStore();
    expect(store.getState().ready).toBe(false);
    expect(store.getState().apply(1, 1, [RENAME])).toBe("gap");
  });

  it("applies the change that comes next and no other", () => {
    const store = started();
    expect(store.getState().apply(3, 11, [RENAME])).toBe("applied");
    expect(store.getState().revision).toBe(11);
    expect(store.getState().tabs["t1"]?.title).toBe("logs");
  });

  it("reports a gap when changes are missing and changes nothing", () => {
    const store = started();
    const before = store.getState();
    expect(store.getState().apply(3, 12, [RENAME])).toBe("gap");
    expect(store.getState()).toBe(before);
  });

  it("drops a change it already has", () => {
    const store = started();
    const before = store.getState();
    expect(store.getState().apply(3, 10, [RENAME])).toBe("stale");
    expect(store.getState().apply(3, 4, [RENAME])).toBe("stale");
    expect(store.getState()).toBe(before);
  });

  it("drops a change to a layout it has left behind", () => {
    const store = started();
    const before = store.getState();
    expect(store.getState().apply(2, 11, [RENAME])).toBe("stale");
    expect(store.getState().apply(2, 500, [RENAME])).toBe("stale");
    expect(store.getState()).toBe(before);
  });

  it("reports a gap for a change to a layout it has not been given", () => {
    const store = started();
    const before = store.getState();
    expect(store.getState().apply(4, 1, [RENAME])).toBe("gap");
    expect(store.getState()).toBe(before);
  });

  it("begins again from a layout taken whole", () => {
    const store = started();
    store.getState().reset(viewOf(oneTab(pane("p9"), "p9"), "w1", 4, 0));
    expect(Object.keys(store.getState().panes)).toEqual(["p9"]);
    expect(Object.keys(store.getState().splits)).toEqual([]);
    expect(store.getState().apply(4, 1, [RENAME])).toBe("applied");
  });
});

describe("what a change leaves alone", () => {
  it("replaces one split's entry when its ratios change", () => {
    const store = started();
    const before = store.getState();
    store.getState().apply(3, 11, [{ type: "ratios", split: "s2", ratios: [300_000, 700_000] }]);
    const after = store.getState();
    expect(after.splits["s2"]?.ratios).toEqual([300_000, 700_000]);
    expect(after.splits["s1"]).toBe(before.splits["s1"]);
    expect(after.panes).toBe(before.panes);
    expect(after.tabs).toBe(before.tabs);
    expect(after.windows).toBe(before.windows);
    expect(after.paneTabs).toBe(before.paneTabs);
  });

  it("replaces only the window's entry when the focus moves", () => {
    const store = started();
    const before = store.getState();
    store.getState().apply(3, 11, [{ type: "focus", window: "w1", pane: "p3" }]);
    const after = store.getState();
    expect(after.windows["w1"]?.focus).toBe("p3");
    expect(after.windows["w1"]?.tabs).toBe(before.windows["w1"]?.tabs);
    expect(after.panes).toBe(before.panes);
    expect(after.tabs).toBe(before.tabs);
    expect(after.splits).toBe(before.splits);
    expect(after.lastFocus["t1"]).toBe("p3");
  });

  it("replaces one pane's entry when the pane changes", () => {
    const store = started();
    const before = store.getState();
    store.getState().apply(3, 11, [{ type: "pane", pane: { ...paneOf("p2"), title: "build" } }]);
    const after = store.getState();
    expect(after.panes["p2"]?.title).toBe("build");
    expect(after.panes["p1"]).toBe(before.panes["p1"]);
    expect(after.panes["p3"]).toBe(before.panes["p3"]);
    expect(after.tabs).toBe(before.tabs);
    expect(after.splits).toBe(before.splits);
  });

  it("keeps the panes and splits a new tree still holds as they were", () => {
    const store = started();
    const before = store.getState();
    // p3 is closed: p2 takes the column's place in the row.
    const root = split("s1", "row", [500_000, 500_000], [pane("p1"), pane("p2")]);
    store.getState().apply(3, 11, [{ type: "tab_tree", tab: "t1", root, zoomed: null }]);
    const after = store.getState();
    expect(after.panes["p1"]).toBe(before.panes["p1"]);
    expect(after.panes["p2"]).toBe(before.panes["p2"]);
    expect(after.splits["s1"]).toBe(before.splits["s1"]);
    expect(after.panes["p3"]).toBeUndefined();
    expect(after.splits["s2"]).toBeUndefined();
    expect(after.paneTabs["p3"]).toBeUndefined();
  });

  it("keeps a tab's entry when a tree of the same shape arrives with a zoom", () => {
    const store = started();
    const before = store.getState();
    const root = threePanes().windows[0]!.centre.tabs[0]!.root;
    store.getState().apply(3, 11, [{ type: "tab_tree", tab: "t1", root, zoomed: "p2" }]);
    const after = store.getState();
    expect(after.tabs["t1"]?.zoomed).toBe("p2");
    expect(after.tabs["t1"]?.root).toBe(before.tabs["t1"]?.root);
    expect(after.panes).toBe(before.panes);
    expect(after.splits).toBe(before.splits);
  });

  it("passes over a change about something it does not have", () => {
    const store = started();
    const before = toWorkspace(store.getState());
    const changes: LayoutChange[] = [
      { type: "ratios", split: "s77", ratios: [1, 2] },
      { type: "tab_title", tab: "t77", title: "x" },
      { type: "focus", window: "w77", pane: "p1" },
      { type: "float_rect", pane: "p77", rect: { x: 0, y: 0, w: 1, h: 1 } },
      { type: "pane", pane: paneOf("p77") },
    ];
    expect(store.getState().apply(3, 11, changes)).toBe("applied");
    expect(toWorkspace(store.getState())).toEqual(before);
  });
});

describe("a pane that moves between a tab and the floating layer", () => {
  const rect = { x: 200_000, y: 200_000, w: 600_000, h: 600_000 };
  const tree: LayoutChange = {
    type: "tab_tree",
    tab: "t1",
    root: split("s1", "row", [500_000, 500_000], [pane("p1"), pane("p2")]),
    zoomed: null,
  };
  const floating: LayoutChange = {
    type: "floating",
    window: "w1",
    floating: [{ pane: paneOf("p3"), rect, minimised: null }],
  };

  it("is kept whichever of the two changes arrives first", () => {
    for (const changes of [
      [tree, floating],
      [floating, tree],
    ]) {
      const store = started();
      store.getState().apply(3, 11, changes);
      const state = store.getState();
      expect(state.panes["p3"]).toBeDefined();
      expect(state.floats["p3"]).toEqual({ window: "w1", rect, minimised: null });
      expect(state.paneTabs["p3"]).toBeUndefined();
      expect(state.windows["w1"]?.floating).toEqual(["p3"]);
    }
  });

  it("is put away and brought back without losing its place", () => {
    const store = started();
    store.getState().apply(3, 11, [tree, floating]);
    const shown = store.getState();
    expect(shown.windows["w1"]?.minimised).toEqual([]);
    expect(isPaneVisible(shown, "p3")).toBe(true);

    const away: LayoutChange = {
      type: "floating",
      window: "w1",
      floating: [{ pane: paneOf("p3"), rect, minimised: "tab" }],
    };
    store.getState().apply(3, 12, [away]);
    const hidden = store.getState();
    expect(hidden.floats["p3"]).toEqual({ window: "w1", rect, minimised: "tab" });
    expect(hidden.windows["w1"]?.floating).toEqual(["p3"]);
    expect(selectMinimised(hidden)).toEqual(["p3"]);
    expect(isMinimised(hidden, "p3")).toBe(true);
    expect(isPaneVisible(hidden, "p3")).toBe(false);
    expect(selectVisiblePanes(hidden)).not.toContain("p3");
    // The pane itself is kept: its terminal carries on while it is put away.
    expect(hidden.panes["p3"]).toBe(shown.panes["p3"]);
    expect(toWorkspace(hidden).windows[0]?.floating[0]?.minimised).toBe("tab");

    store.getState().apply(3, 13, [floating]);
    const back = store.getState();
    expect(selectMinimised(back)).toEqual([]);
    // A window with nothing put away has the same list each time.
    expect(selectMinimised(back)).toBe(selectMinimised(shown));
    expect(isPaneVisible(back, "p3")).toBe(true);
  });

  it("comes back into the tab whichever change arrives first", () => {
    const back: LayoutChange = {
      type: "tab_tree",
      tab: "t1",
      root: threePanes().windows[0]!.centre.tabs[0]!.root,
      zoomed: null,
    };
    const none: LayoutChange = { type: "floating", window: "w1", floating: [] };
    for (const changes of [
      [back, none],
      [none, back],
    ]) {
      const store = started();
      store.getState().apply(3, 11, [tree, floating]);
      store.getState().apply(3, 12, changes);
      const state = store.getState();
      expect(state.panes["p3"]).toBeDefined();
      expect(state.floats["p3"]).toBeUndefined();
      expect(state.paneTabs["p3"]).toBe("t1");
      expect(toWorkspace(state)).toEqual(followerForm(threePanes()));
    }
  });
});

function paneOf(id: string) {
  const node = pane(id);
  if (node.type !== "pane") throw new Error("not a pane");
  return { id: node.id, title: node.title, content: node.content };
}

describe("the workspace's name", () => {
  it("follows the workspace when it is renamed, and nothing else changes", () => {
    const store = started();
    const before = store.getState();
    expect(before.name).toBe("test");
    expect(store.getState().apply(3, 11, [{ type: "workspace_name", name: "notes" }])).toBe(
      "applied",
    );
    const after = store.getState();
    expect(after.name).toBe("notes");
    expect(after.workspaceId).toBe(before.workspaceId);
    expect(after.panes).toBe(before.panes);
    expect(after.tabs).toBe(before.tabs);
    expect(after.windows).toBe(before.windows);
    expect(toWorkspace(after).name).toBe("notes");
  });
});
