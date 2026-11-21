import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { STATUS_FIXTURES } from "../src/status/fixtures.ts";
import { createStatusStore } from "../src/status/registry.ts";
import { sparklinePoints, StatusItemView, StatusList } from "../src/status/StatusItem.tsx";
import { STATUS_KINDS, type StatusItem, type TextItem } from "../src/status/types.ts";

function markup(item: StatusItem): string {
  return renderToStaticMarkup(createElement(StatusItemView, { item }));
}

function fixture(kind: StatusItem["kind"]): StatusItem {
  const found = STATUS_FIXTURES.find((item) => item.kind === kind);
  if (!found) throw new Error(`the fixtures have no ${kind}`);
  return found;
}

describe("status items drawn from the fixtures", () => {
  it("has a fixture of every kind", () => {
    expect([...new Set(STATUS_FIXTURES.map((item) => item.kind))].sort()).toEqual(
      // A segmented item is a control of a pane's header, which the bars' fixtures
      // have none of.
      STATUS_KINDS.filter((kind) => kind !== "segmented").sort(),
    );
  });

  it("marks every item with its id, its kind and its tone", () => {
    for (const item of STATUS_FIXTURES) {
      const html = markup(item);
      expect(html, item.id).toContain(`data-status="${item.id}"`);
      expect(html, item.id).toContain(`data-kind="${item.kind}"`);
      expect(html, item.id).toContain(`data-tone="${item.tone ?? "neutral"}"`);
      // Nothing is styled by an attribute of its own.
      expect(html, item.id).not.toContain("style=");
    }
  });

  it("draws text as text", () => {
    const html = markup(fixture("text"));
    expect(html).toContain('<span class="status-value">~/src/lume</span>');
    expect(html).toContain('title="The workspace&#x27;s folder"');
  });

  it("draws an icon with the words it stands for", () => {
    const html = markup(fixture("icon"));
    expect(html).toContain('role="img" aria-label="main"');
    expect(html).toContain("<svg");
    expect(html).toContain('<span class="status-text">main</span>');
  });

  it("draws a meter as a name, a bar filled to its value, and the amount", () => {
    const html = markup(fixture("meter"));
    expect(html).toContain('<span class="status-label">CPU</span>');
    expect(html).toContain('role="meter"');
    expect(html).toContain('aria-valuenow="0.31"');
    expect(html).toContain('class="status-meter-track" width="36" height="4"');
    expect(html).toMatch(/class="status-meter-fill" width="11\.16" height="4"/);
    expect(html).toContain('<span class="status-value">31%</span>');
  });

  it("keeps a meter's bar within its track whatever value it is given", () => {
    const over = markup({ id: "a.b", slot: "topbar.right", kind: "meter", label: "X", value: 7 });
    const under = markup({ id: "a.b", slot: "topbar.right", kind: "meter", label: "X", value: -1 });
    const none = markup({ id: "a.b", slot: "topbar.right", kind: "meter", label: "X", value: NaN });
    expect(over).toContain('class="status-meter-fill" width="36"');
    expect(under).toContain('class="status-meter-fill" width="0"');
    expect(none).toContain('class="status-meter-fill" width="0"');
  });

  it("draws a sparkline as a line through its values", () => {
    const html = markup(fixture("sparkline"));
    expect(html).toContain('aria-label="Load: 0.8"');
    const points = /<polyline points="([^"]+)"/.exec(html)?.[1]?.split(" ") ?? [];
    expect(points).toHaveLength(8);
    // The first value at the left, the last at the right, the highest at the top.
    expect(points[0]).toBe("1.5,12.5");
    expect(points[5]?.endsWith(",1.5")).toBe(true);
    expect(points[7]?.startsWith("46.5,")).toBe(true);
  });

  it("draws a badge as a small box", () => {
    expect(markup(fixture("badge"))).toContain('<span class="status-badge">3</span>');
  });

  it("draws a segmented item as a group of buttons, the chosen one pressed", () => {
    const html = markup({
      id: "a.view",
      slot: "pane.header",
      kind: "segmented",
      label: "Pane view",
      choices: [
        { value: "terminal", label: "Terminal" },
        { value: "ui", label: "UI" },
      ],
      selected: "ui",
      command: "a.show-as",
    });
    expect(html).toContain('role="group"');
    expect(html).toContain('aria-label="Pane view"');
    const parts = [
      ...html.matchAll(/<button[^>]*aria-pressed="(true|false)"[^>]*aria-label="([^"]+)"/g),
    ];
    expect(parts.map((part) => [part[2], part[1]])).toEqual([
      ["Terminal", "false"],
      ["UI", "true"],
    ]);
  });

  it("writes text a program might send as text, never as markup", () => {
    const html = markup({
      id: "a.b",
      slot: "topbar.left",
      kind: "text",
      text: "<img src=x onerror=alert(1)>",
    });
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;img");
  });
});

describe("the points of a sparkline", () => {
  it("has no points for no values", () => {
    expect(sparklinePoints([])).toBe("");
    expect(sparklinePoints([NaN, Infinity])).toBe("");
  });

  it("runs along the middle when the values are all alike", () => {
    expect(sparklinePoints([3, 3, 3])).toBe("1.5,7.0 24.0,7.0 46.5,7.0");
    expect(sparklinePoints([5])).toBe("1.5,7.0");
  });
});

describe("the registry of status items", () => {
  const text = (id: string, order?: number): TextItem => ({
    id,
    slot: "topbar.left",
    kind: "text",
    text: id,
    order,
  });

  it("keeps each slot's items in order, and items alike in order by their ids", () => {
    const store = createStatusStore();
    for (const item of STATUS_FIXTURES) store.getState().register(item);
    const ids = (slot: StatusItem["slot"]) => store.getState().bySlot[slot].map((item) => item.id);
    expect(ids("topbar.left")).toEqual(["fixture.branch", "fixture.directory"]);
    expect(ids("topbar.center")).toEqual(["fixture.waiting"]);
    expect(ids("topbar.right")).toEqual(["fixture.cpu", "fixture.memory", "fixture.load"]);
    expect(ids("pane.header")).toEqual([]);

    store.getState().register(text("b.one"));
    store.getState().register(text("a.one"));
    expect(ids("topbar.left").slice(0, 2)).toEqual(["a.one", "b.one"]);
  });

  it("replaces an item registered again under the same id, wherever it was", () => {
    const store = createStatusStore();
    store.getState().register(text("a.one"));
    store.getState().register({ ...text("a.one"), slot: "topbar.right", text: "moved" });
    expect(store.getState().bySlot["topbar.left"]).toEqual([]);
    expect(store.getState().bySlot["topbar.right"]).toHaveLength(1);
    expect(store.getState().find("a.one")).toMatchObject({ text: "moved" });
  });

  it("updates an item that is shown and nothing else", () => {
    const store = createStatusStore();
    store.getState().register(fixture("meter"));
    expect(store.getState().update("fixture.cpu", { value: 0.9, text: "90%" })).toBe(true);
    expect(store.getState().find("fixture.cpu")).toMatchObject({
      kind: "meter",
      value: 0.9,
      text: "90%",
      label: "CPU",
    });
    expect(store.getState().update("no.such", { tooltip: "x" })).toBe(false);
  });

  it("leaves the lists of the other slots as they were when one slot changes", () => {
    const store = createStatusStore();
    for (const item of STATUS_FIXTURES) store.getState().register(item);
    const before = store.getState().bySlot;
    store.getState().update("fixture.cpu", { value: 0.5 });
    const after = store.getState().bySlot;
    expect(after["topbar.right"]).not.toBe(before["topbar.right"]);
    expect(after["topbar.left"]).toBe(before["topbar.left"]);
    expect(after["topbar.center"]).toBe(before["topbar.center"]);
    expect(after["pane.header"]).toBe(before["pane.header"]);
  });

  it("removes an item, and leaves the registry alone for one it does not have", () => {
    const store = createStatusStore();
    store.getState().register(text("a.one"));
    const before = store.getState();
    store.getState().remove("no.such");
    expect(store.getState()).toBe(before);
    store.getState().remove("a.one");
    expect(store.getState().bySlot["topbar.left"]).toEqual([]);
    expect(store.getState().find("a.one")).toBeUndefined();
  });

  it("renders a slot's items in order, and a pane's header only its own", () => {
    const store = createStatusStore();
    for (const item of STATUS_FIXTURES) store.getState().register(item);
    const list = (slot: StatusItem["slot"], pane?: string) =>
      renderToStaticMarkup(
        createElement(StatusList, { items: store.getState().bySlot[slot], slot, pane }),
      );
    const right = list("topbar.right");
    expect(right).toContain('data-slot="topbar.right"');
    expect(right.indexOf("fixture.cpu")).toBeLessThan(right.indexOf("fixture.memory"));
    expect(right.indexOf("fixture.memory")).toBeLessThan(right.indexOf("fixture.load"));

    expect(list("pane.header")).toBe("");
    store.getState().register({
      id: "a.exit",
      slot: "pane.header",
      kind: "badge",
      text: "1",
      pane: "p2",
    });
    expect(list("pane.header", "p2")).toContain("a.exit");
    expect(list("pane.header", "p1")).toBe("");
  });
});
