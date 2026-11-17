import { describe, expect, it } from "vitest";

import {
  dropPlace,
  groupDropIntent,
  joinedGroup,
  tabDropIntent,
  type DropBox,
  type RowTab,
} from "../src/shell/tab-drop.ts";

/** t1 | [g1: label, t2, t3] | t4 | [g2: label, t5] */
const ROW: readonly RowTab[] = [
  { tab: "t1", group: null },
  { tab: "t2", group: "g1" },
  { tab: "t3", group: "g1" },
  { tab: "t4", group: null },
  { tab: "t5", group: "g2" },
];

/** The same row laid out, each thing a hundred pixels wide but the labels, forty. */
function boxes(folded: ReadonlySet<string> = new Set()): DropBox[] {
  const order: readonly [string, string | null][] = [
    ["t1", null],
    ["Lg1", "g1"],
    ["t2", "g1"],
    ["t3", "g1"],
    ["t4", null],
    ["Lg2", "g2"],
    ["t5", "g2"],
  ];
  let at = 0;
  return order.map(([id, group]) => {
    const width = id.startsWith("L") ? 40 : folded.has(id) ? 0 : 100;
    const box = { id, group, start: at, end: at + width, folded: folded.has(id) };
    at += width;
    return box;
  });
}

describe("where a dragged tab lands", () => {
  it("lands before or after a tab by the tab's middle", () => {
    expect(dropPlace(boxes(), "t1", 260)).toEqual({ target: "t3", side: "before" });
    expect(dropPlace(boxes(), "t1", 320)).toEqual({ target: "t3", side: "after" });
  });

  it("joins a group at its start when let go on its label", () => {
    expect(dropPlace(boxes(), "t4", 110)).toEqual({ target: "Lg1", side: "after" });
  });

  it("keeps the place it had between two things and over nothing", () => {
    expect(dropPlace(boxes(), "t1", 10_000)).toBeUndefined();
  });

  it("passes over a folded tab", () => {
    const folded = new Set(["t5"]);
    expect(dropPlace(boxes(folded), "t1", 480)).toBeUndefined();
  });
});

describe("where a dragged group lands", () => {
  it("lands before or after another group as a whole", () => {
    // Over g1's first tab, which is in the first half of g1: before g1's label.
    expect(dropPlace(boxes(), "Lg2", 150)).toEqual({ target: "Lg1", side: "before" });
    // Over g1's last tab, past g1's middle: after g1's last tab.
    expect(dropPlace(boxes(), "Lg2", 300)).toEqual({ target: "t3", side: "after" });
  });

  it("does not land on itself", () => {
    expect(dropPlace(boxes(), "Lg2", 500)).toBeUndefined();
  });

  it("lands after a folded group's label", () => {
    const folded = new Set(["t2", "t3"]);
    expect(dropPlace(boxes(folded), "Lg2", 130)).toEqual({ target: "Lg1", side: "after" });
  });
});

describe("which group a tab joins", () => {
  it("joins a group between two of its tabs", () => {
    expect(joinedGroup(ROW, "t1", "t3", "before")).toBe("g1");
    expect(joinedGroup(ROW, "t1", "t2", "after")).toBe("g1");
  });

  it("stays out of a group just outside it", () => {
    expect(joinedGroup(ROW, "t1", "t2", "before")).toBeNull();
    expect(joinedGroup(ROW, "t1", "t3", "after")).toBeNull();
  });

  it("stays in its own group anywhere among its tabs", () => {
    expect(joinedGroup(ROW, "t2", "t3", "after")).toBe("g1");
  });

  it("joins a group let go on its label", () => {
    expect(joinedGroup(ROW, "t4", "Lg2", "after")).toBe("g2");
  });
});

describe("what letting go asks of the layout", () => {
  it("puts a tab at its place and in its group", () => {
    expect(tabDropIntent(ROW, "t1", { target: "t3", side: "before" })).toEqual({
      type: "tab_place",
      tab: "t1",
      index: 1,
      group: "g1",
    });
    expect(tabDropIntent(ROW, "t4", { target: "Lg2", side: "after" })).toEqual({
      type: "tab_place",
      tab: "t4",
      index: 3,
      group: "g2",
    });
  });

  it("takes a tab out of its group without moving it", () => {
    expect(tabDropIntent(ROW, "t3", { target: "t4", side: "before" })).toEqual({
      type: "tab_place",
      tab: "t3",
      index: 2,
      group: null,
    });
  });

  it("asks nothing of a tab let go where it is", () => {
    expect(tabDropIntent(ROW, "t2", { target: "t3", side: "before" })).toBeNull();
    expect(tabDropIntent(ROW, "t2", { target: "t2", side: "before" })).toBeNull();
  });

  it("moves a group to a place among the tabs that are not in it", () => {
    expect(groupDropIntent(ROW, "g2", { target: "Lg1", side: "before" })).toEqual({
      type: "group_move",
      group: "g2",
      index: 1,
    });
    expect(groupDropIntent(ROW, "g1", { target: "t4", side: "after" })).toEqual({
      type: "group_move",
      group: "g1",
      index: 2,
    });
  });

  it("never puts a group inside another, nor where it is", () => {
    expect(groupDropIntent(ROW, "g2", { target: "t2", side: "after" })).toBeNull();
    expect(groupDropIntent(ROW, "g1", { target: "t1", side: "after" })).toBeNull();
  });
});
