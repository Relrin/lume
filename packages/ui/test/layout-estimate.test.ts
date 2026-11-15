import type { CellMetrics } from "@lume/renderer";
import { describe, expect, it } from "vitest";

import { estimatePane, gridFor, shapeOf } from "../src/layout/estimate.ts";
import { computeRects, type Rect } from "../src/layout/geometry.ts";
import { createLayoutStore, type LayoutState } from "../src/layout/store.ts";
import { UNITS, type Intent, type Node, type Workspace } from "../src/layout/types.ts";
import { MAX_DIMENSION } from "../src/transport/types.ts";
import { oneTab, pane, split, vector, viewOf } from "./layout-support.ts";

const BOX: Rect = { x: 8, y: 44, width: 1003, height: 611 };
const GAP = 8;
const TERMINAL = { type: "terminal" } as const;

function stateOf(workspace: Workspace): LayoutState {
  const store = createLayoutStore();
  store.getState().reset(viewOf(workspace));
  return store.getState();
}

function splitOf(target: string, dir: "left" | "right" | "up" | "down", ratio: number): Intent {
  return { type: "split", pane: target, dir, ratio, content: TERMINAL };
}

function placed(root: Node, id: string): Rect | undefined {
  return computeRects(root, BOX, GAP).panes.find((entry) => entry.pane === id)?.rect;
}

describe("where a split would put its new pane", () => {
  it("is where the tree lays it out once the pane is there", () => {
    const state = stateOf(oneTab(pane("p1")));
    const cases: [Intent, Node][] = [
      [
        splitOf("p1", "right", 300_000),
        split("s9", "row", [700_000, 300_000], [pane("p1"), pane("new")]),
      ],
      [
        splitOf("p1", "left", 300_000),
        split("s9", "row", [300_000, 700_000], [pane("new"), pane("p1")]),
      ],
      [
        splitOf("p1", "down", 450_000),
        split("s9", "column", [550_000, 450_000], [pane("p1"), pane("new")]),
      ],
      [
        splitOf("p1", "up", 450_000),
        split("s9", "column", [450_000, 550_000], [pane("new"), pane("p1")]),
      ],
    ];
    for (const [intent, after] of cases) {
      const estimated = estimatePane(state, intent, BOX, GAP);
      expect(estimated).toBeDefined();
      expect(estimated).toEqual(placed(after, "new"));
    }
  });

  it("divides across the row's direction inside the pane's own space", () => {
    const before = split("s1", "row", [400_000, 600_000], [pane("p1"), pane("p2")]);
    const after = split(
      "s1",
      "row",
      [400_000, 600_000],
      [pane("p1"), split("s9", "column", [500_000, 500_000], [pane("p2"), pane("new")])],
    );
    const estimated = estimatePane(
      stateOf(oneTab(before)),
      splitOf("p2", "down", UNITS / 2),
      BOX,
      GAP,
    );
    expect(estimated).toEqual(placed(after, "new"));
  });

  it("joins the row it is in when it divides along the row", () => {
    // p2 has 600 000 of the row; the new pane takes a third of that, and the three
    // then share the row, which has one more gap than before.
    const before = split("s1", "row", [400_000, 600_000], [pane("p1"), pane("p2")]);
    const after = split(
      "s1",
      "row",
      [400_000, 400_000, 200_000],
      [pane("p1"), pane("p2"), pane("new")],
    );
    const intent = splitOf("p2", "right", Math.round(UNITS / 3));
    const estimated = estimatePane(stateOf(oneTab(before)), intent, BOX, GAP);
    const expected = placed(after, "new");
    expect(estimated?.y).toBe(expected?.y);
    expect(estimated?.height).toBe(expected?.height);
    // A third is not a whole number of units, so the two may differ by a pixel.
    expect(Math.abs(estimated!.width - expected!.width)).toBeLessThanOrEqual(1);
    expect(estimated!.x + estimated!.width).toBe(BOX.x + BOX.width);
  });

  it("agrees with the core's own layouts in the recorded sequences", () => {
    const recorded = vector("splits-and-tabs");
    const store = createLayoutStore();
    store.getState().reset(viewOf(recorded.start));
    let revision = 0;
    let compared = 0;
    for (const step of recorded.steps) {
      if (!step.changes || step.changes.length === 0) continue;
      const estimated = estimatePane(store.getState(), step.intent, BOX, GAP);
      store.getState().apply(1, ++revision, step.changes);
      const made = step.created?.pane;
      if (step.intent.type !== "split" || !made) continue;
      // The new pane as the core placed it in the tab's tree.
      const state = store.getState();
      const tab = state.tabs[state.paneTabs[made]!]!;
      const actual = computeRects(shapeOf(state, tab.root), BOX, GAP).panes.find(
        (entry) => entry.pane === made,
      );
      expect(estimated).toEqual(actual?.rect);
      compared += 1;
    }
    expect(compared).toBeGreaterThanOrEqual(2);
  });

  it("is laid out whole even while another pane is zoomed", () => {
    const workspace = oneTab(split("s1", "row", [500_000, 500_000], [pane("p1"), pane("p2")]));
    const zoomed: Workspace = {
      ...workspace,
      windows: workspace.windows.map((window) => ({
        ...window,
        centre: {
          ...window.centre,
          tabs: window.centre.tabs.map((tab) => ({ ...tab, zoomed: "p1" })),
        },
      })),
    };
    const intent = splitOf("p1", "down", UNITS / 2);
    expect(estimatePane(stateOf(zoomed), intent, BOX, GAP)).toEqual(
      estimatePane(stateOf(workspace), intent, BOX, GAP),
    );
  });

  it("is nothing for a pane that is not in a tab, or a share no pane may have", () => {
    const state = stateOf(oneTab(pane("p1")));
    expect(estimatePane(state, splitOf("p9", "right", UNITS / 2), BOX, GAP)).toBeUndefined();
    expect(estimatePane(state, splitOf("p1", "right", 10), BOX, GAP)).toBeUndefined();
    expect(estimatePane(state, splitOf("p1", "right", UNITS), BOX, GAP)).toBeUndefined();
  });
});

describe("where other intents would put a pane", () => {
  const state = stateOf(oneTab(pane("p1")));

  it("gives a new tab and a new window the whole box", () => {
    expect(
      estimatePane(state, { type: "tab_new", window: "w1", content: TERMINAL }, BOX, GAP),
    ).toEqual(BOX);
    expect(estimatePane(state, { type: "window_new", content: TERMINAL }, BOX, GAP)).toEqual(BOX);
  });

  it("floats the first scratch terminal in the middle", () => {
    const intent: Intent = { type: "scratch_show", window: "w1", content: TERMINAL };
    const estimated = estimatePane(state, intent, { x: 0, y: 0, width: 1000, height: 500 }, GAP);
    expect(estimated).toEqual({ x: 200, y: 100, width: 600, height: 300 });
  });

  it("makes no scratch terminal when there is one, or nothing to make it from", () => {
    const recorded = vector("floating-and-scratch");
    const withScratch = stateOf(recorded.end);
    if (withScratch.scratch !== null) {
      const again: Intent = { type: "scratch_show", window: "w1", content: TERMINAL };
      expect(estimatePane(withScratch, again, BOX, GAP)).toBeUndefined();
    }
    expect(estimatePane(state, { type: "scratch_show", window: "w1" }, BOX, GAP)).toBeUndefined();
    expect(
      estimatePane(state, { type: "scratch_show", window: "w9", content: TERMINAL }, BOX, GAP),
    ).toBeUndefined();
  });

  it("is nothing for an intent that makes no pane", () => {
    for (const intent of [
      { type: "close", pane: "p1" },
      { type: "zoom", pane: "p1", on: true },
      { type: "float", pane: "p1" },
      { type: "scratch_hide" },
    ] satisfies Intent[]) {
      expect(estimatePane(state, intent, BOX, GAP)).toBeUndefined();
    }
  });
});

describe("the grid of a terminal in a card", () => {
  const metrics: CellMetrics = {
    cellWidth: 8,
    cellHeight: 17,
    baseline: 13,
    lineThickness: 1,
    underlineTop: 14,
    strikethroughTop: 8,
    padding: 4,
  };
  const chrome = { headerPx: 34, borderPx: 1 };

  it("is the whole cells that fit under the header and inside the borders", () => {
    // 500 - 2 = 498 px wide, less 8 of padding: 61 cells of 8. 300 - 34 - 2 = 264 px
    // high, less 8: 15 cells of 17.
    const card = { x: 0, y: 0, width: 500, height: 300 };
    expect(gridFor(card, chrome, metrics, 1)).toEqual({ cols: 61, rows: 15 });
    expect(gridFor(card, { headerPx: 0, borderPx: 0 }, metrics, 1)).toEqual({ cols: 61, rows: 17 });
  });

  it("counts in device pixels", () => {
    const doubled = { ...metrics, cellWidth: 16, cellHeight: 34, padding: 8 };
    const card = { x: 0, y: 0, width: 500, height: 300 };
    expect(gridFor(card, chrome, doubled, 2)).toEqual({ cols: 61, rows: 15 });
    // 498 px at one and a half is 747 device pixels: 92 cells of 8 after the padding.
    expect(gridFor(card, chrome, metrics, 1.5).cols).toBe(92);
  });

  it("is never less than one cell nor more than a terminal may have", () => {
    expect(gridFor({ x: 0, y: 0, width: 3, height: 3 }, chrome, metrics, 1)).toEqual({
      cols: 1,
      rows: 1,
    });
    const huge = { x: 0, y: 0, width: 1e6, height: 1e6 };
    expect(gridFor(huge, chrome, metrics, 1)).toEqual({ cols: MAX_DIMENSION, rows: MAX_DIMENSION });
  });
});
