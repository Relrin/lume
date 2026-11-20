import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import {
  selectGroups,
  selectGroupTabs,
  selectShelf,
  selectStripKey,
  stripItems,
} from "../src/layout/selectors.ts";
import { createLayoutStore, toWorkspace } from "../src/layout/store.ts";
import type { Group, LayoutChange, Shelved, Tab, Workspace } from "../src/layout/types.ts";
import { panesClosedBy } from "../src/shell/closing.ts";
import { Menu, type MenuItem } from "../src/shell/Menu.tsx";
import { oneTab, pane, PlainFollower, viewOf } from "./layout-support.ts";

const BILLING: Group = { id: "g9", name: "Billing", color: "teal", icon: "bug" };

function tab(id: string, extra: Partial<Tab> = {}): Tab {
  return {
    id,
    title: null,
    root: pane(`p${id.slice(1)}`),
    zoomed: null,
    last_focus: null,
    ...extra,
  };
}

/** Four tabs: t1 alone, t2 and t3 in Billing, t4 alone and locked. */
function grouped(): Workspace {
  const base = oneTab(pane("p1"), "p1");
  const window = base.windows[0];
  if (!window) throw new Error("the window is there");
  return {
    ...base,
    windows: [
      {
        ...window,
        centre: {
          tabs: [
            tab("t1", { last_focus: "p1" }),
            tab("t2", { group: "g9", color: "red" }),
            tab("t3", { group: "g9" }),
            tab("t4", { locked: true }),
          ],
          active: "t1",
          groups: [BILLING],
        },
      },
    ],
  };
}

function started() {
  const store = createLayoutStore();
  store.getState().reset(viewOf(grouped(), "w1", 1, 1));
  return store;
}

const SHELVED: Shelved = {
  group: { ...BILLING, id: "g12", kept: true },
  tabs: [tab("t20", { group: "g12" })],
};

describe("a layout store with groups of tabs", () => {
  it("takes the tabs' colours, locks and groups, and the window's groups", () => {
    const state = started().getState();
    expect(state.tabs["t2"]).toMatchObject({ color: "red", locked: false, group: "g9" });
    expect(state.tabs["t4"]).toMatchObject({ color: null, locked: true, group: null });
    expect(selectGroups(state)).toEqual([BILLING]);
    expect(selectGroupTabs(state, "g9")).toEqual(["t2", "t3"]);
    expect(selectShelf(state)).toEqual([]);
  });

  it("gives the workspace back as the core writes it", () => {
    const state = started().getState();
    const plain = new PlainFollower(grouped());
    expect(toWorkspace(state)).toEqual(plain.workspace);
  });

  it("follows a tab's state, the window's groups and the shelf", () => {
    const store = started();
    const changes: LayoutChange[] = [
      { type: "tab_state", tab: "t1", color: "blue", locked: true, group: "g9" },
      { type: "groups", window: "w1", groups: [{ ...BILLING, collapsed: true }] },
      { type: "shelf", shelf: [SHELVED] },
    ];
    const plain = new PlainFollower(grouped());
    for (const change of changes) plain.apply(change);
    expect(store.getState().apply(1, 2, changes)).toBe("applied");
    const state = store.getState();
    expect(state.tabs["t1"]).toMatchObject({ color: "blue", locked: true, group: "g9" });
    expect(selectGroups(state)[0]?.collapsed).toBe(true);
    expect(selectShelf(state)).toEqual([SHELVED]);
    expect(toWorkspace(state)).toEqual(plain.workspace);
  });

  it("keeps an entry that a change leaves as it was", () => {
    const store = started();
    const before = store.getState();
    expect(
      store.getState().apply(1, 2, [
        { type: "tab_state", tab: "t2", color: "red", locked: false, group: "g9" },
        { type: "groups", window: "w1", groups: [BILLING] },
      ]),
    ).toBe("applied");
    expect(store.getState().tabs["t2"]).toBe(before.tabs["t2"]);
    expect(store.getState().windows["w1"]).toBe(before.windows["w1"]);
  });

  it("lays the row out with each group's label before its tabs", () => {
    const state = started().getState();
    const items = stripItems(state);
    expect(items.map((item) => (item.kind === "group" ? `[${item.group.id}` : item.tab))).toEqual([
      "t1",
      "[g9",
      "t2",
      "t3",
      "t4",
    ]);
    const last = items.find((item) => item.kind === "tab" && item.tab === "t3");
    expect(last).toMatchObject({ kind: "tab", last: true, folded: false });
  });

  it("says when the row changes, and only then", () => {
    const store = started();
    const key = selectStripKey(store.getState());
    store.getState().apply(1, 2, [{ type: "tab_title", tab: "t1", title: "logs" }]);
    expect(selectStripKey(store.getState())).toBe(key);
    store
      .getState()
      .apply(1, 3, [{ type: "groups", window: "w1", groups: [{ ...BILLING, collapsed: true }] }]);
    expect(selectStripKey(store.getState())).not.toBe(key);
    const folded = stripItems(store.getState()).filter(
      (item) => item.kind === "tab" && item.folded,
    );
    expect(folded.length).toBe(2);
  });
});

describe("closing tabs and groups", () => {
  it("leaves a locked tab out", () => {
    const state = started().getState();
    expect(panesClosedBy(state, "lume.tab.close", { tab: "t4" })).toBeUndefined();
    expect(panesClosedBy(state, "lume.tab.close-right", { tab: "t1" })).toEqual({
      what: "tabs",
      panes: ["p2", "p3"],
    });
    expect(panesClosedBy(state, "lume.tab.close-left", { tab: "t1" })).toBeUndefined();
  });

  it("closes a group's tabs, and moves them all", () => {
    const state = started().getState();
    expect(panesClosedBy(state, "lume.group.close", { group: "g9" })).toEqual({
      what: "group",
      panes: ["p2", "p3"],
    });
    expect(
      panesClosedBy(state, "lume.group.move-to-workspace", { group: "g9", workspace: "x" }),
    ).toEqual({ what: "moved-group", panes: ["p2", "p3"] });
    // The tab on show is in no group: there is no group to close.
    expect(panesClosedBy(state, "lume.group.close", null)).toBeUndefined();
  });
});

describe("a menu's rows besides items", () => {
  const nothing = () => undefined;

  it("draws a row of swatches, a heading and a note", () => {
    const items: MenuItem[] = [
      {
        key: "color",
        kind: "swatches",
        label: "Color",
        swatches: [
          { key: "none", label: "No color", color: null, checked: false, choose: nothing },
          { key: "red", label: "Red", color: "var(--red)", checked: true, choose: nothing },
        ],
        choose: nothing,
      },
      { key: "to", kind: "heading", label: "Add to group", choose: nothing },
      { key: "g", label: "Billing", choose: nothing },
      { key: "why", kind: "note", label: "Says why.", choose: nothing },
    ];
    const html = renderToStaticMarkup(createElement(Menu, { label: "Tab", items, close: nothing }));
    expect(html).toContain('class="menu-swatches" role="group" aria-label="Color"');
    expect(html).toMatch(
      /role="menuitemradio" aria-checked="false" aria-label="No color"[^>]*data-none="true"/,
    );
    expect(html).toMatch(
      /aria-checked="true" aria-label="Red"[^>]*style="background:var\(--red\)"/,
    );
    expect(html).toContain('<div class="menu-heading" role="presentation">Add to group</div>');
    expect(html).toContain('<p class="menu-note" role="presentation">Says why.</p>');
    // A heading or a note is no item.
    expect(html.match(/role="menuitem"/g)?.length).toBe(1);
  });
});

describe("a notice that offers something to do", () => {
  it("keeps what it offers, and is told apart from the same words offering nothing", async () => {
    const { NoticeStore } = await import("../src/shell/notices.ts");
    const store = new NoticeStore();
    const shown: { key: string; action?: unknown }[] = [];
    store.connect({ show: (notice) => void shown.push(notice), hide: () => undefined });
    const undo = { label: "Undo", command: "lume.group.bring-back", args: { group: "g12" } };
    store.push("info", "Moved Billing to acme", undo);
    expect(shown[0]?.action).toEqual(undo);
    store.push("info", "Moved Billing to acme");
    expect(shown.length).toBe(2);
    expect(shown[1]?.action).toBeUndefined();
    expect(shown[1]?.key).not.toBe(shown[0]?.key);
  });
});
