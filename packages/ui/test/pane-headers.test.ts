import { describe, expect, it } from "vitest";

import { createLayoutStore } from "../src/layout/store.ts";
import { PaneViews, type ViewsServices } from "../src/panes/views.ts";

/** Views with nothing behind them: enough to ask about headers. */
function views(): PaneViews {
  const services = {
    layout: createLayoutStore(),
    area: { onBox: () => undefined },
  } as unknown as ViewsServices;
  return new PaneViews(services);
}

describe("which panes show a header", () => {
  it("knows a pane's header while its card shows one", () => {
    const panes = views();
    expect(panes.hasHeader("p1")).toBe(false);
    const gone = panes.keepHeader("p1");
    expect(panes.hasHeader("p1")).toBe(true);
    expect(panes.hasHeader("p2")).toBe(false);
    gone();
    expect(panes.hasHeader("p1")).toBe(false);
  });

  it("keeps a header two cards show until both let go, and counts a letting go once", () => {
    const panes = views();
    const first = panes.keepHeader("p1");
    const second = panes.keepHeader("p1");
    first();
    first();
    expect(panes.hasHeader("p1")).toBe(true);
    second();
    expect(panes.hasHeader("p1")).toBe(false);
  });
});
