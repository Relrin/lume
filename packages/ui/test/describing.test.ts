import { describe, expect, it } from "vitest";
import { createStore } from "zustand/vanilla";

import type { LayoutState } from "../src/layout/store.ts";
import { PaneReports } from "../src/events/store.ts";
import { createRexStore } from "../src/rex/store.ts";
import type { ShellApp } from "../src/shell/boot.ts";
import { followPane, followTab, paneFacts } from "../src/shell/describing.ts";
import { DEVBOX, NAS, rexList } from "./rex-fixtures.ts";

/** A layout with one tab and one pane in it, whose session is on a backend. */
function layoutOf(backend: string | null): LayoutState {
  const pane = {
    id: "p1",
    title: null,
    content: {
      type: "terminal",
      backend,
      session: "s1",
      profile: null,
      launcher: null,
      cwd: "C:\\here",
      env: [],
    },
  };
  return {
    ready: true,
    // The layout is the application's own: the pane's backend is what tells.
    backend: null,
    panes: { p1: pane },
    paneTabs: { p1: "t1" },
    tabs: { t1: { id: "t1", title: null, root: { type: "pane", id: "p1" }, window: "w1" } },
    lastFocus: { t1: "p1" },
    floats: {},
    windows: {},
    scratch: null,
  } as unknown as LayoutState;
}

function appOf(backend: string | null) {
  const rex = createRexStore();
  const app = {
    reports: new PaneReports(() => false),
    layout: createStore(() => layoutOf(backend)),
    profiles: createStore(() => ({ list: null })),
    config: createStore(() => ({ view: null })),
    plugins: createStore(() => ({ shown: null })),
    machine: { name: "here", home: null },
    rex,
  } as unknown as ShellApp;
  return { app, rex };
}

describe("a pane whose session is on a Rex server", () => {
  it("is told by its own backend, in whichever workspace it is", () => {
    const servers = rexList(DEVBOX, NAS);
    const serverOf = (backend: string | null) => paneFacts(layoutOf(backend), "p1", servers).server;
    expect(serverOf(DEVBOX.id)).toBe("devbox");
    expect(serverOf(NAS.id)).toBe("home-nas");
    // One that is not listed any more is a server's all the same.
    expect(serverOf("rex:https://gone")).toBe("Rex");
    expect(paneFacts(layoutOf(DEVBOX.id), "p1", null).server).toBe("Rex");
    for (const own of [null, "local", "daemon"]) expect(serverOf(own)).toBeNull();
  });

  it("is called after its server when the server is known, and is renamed with it", () => {
    const { app, rex } = appOf(DEVBOX.id);
    const places: string[] = [];
    const labels: string[] = [];
    const stops = [
      followPane(app, "p1", ({ where }) => places.push(where)),
      followTab(app, "t1", ({ label }) => labels.push(label)),
    ];
    rex.getState().set(rexList(DEVBOX));
    // How near a server is, and what it holds, changes often and says nothing here.
    rex.getState().set({ ...rexList({ ...DEVBOX, latency_ms: 80 }), revision: 2 });
    rex.getState().set({ ...rexList({ ...DEVBOX, label: "build" }), revision: 3 });
    expect(places).toEqual(["Rex", "devbox", "build"]);
    expect(labels).toEqual(["Terminal · Rex", "Terminal · devbox", "Terminal · build"]);
    stops.forEach((stop) => stop());
    rex.getState().set({ ...rexList(NAS), revision: 4 });
    expect(places).toHaveLength(3);
  });

  it("is not, where its session is here", () => {
    const { app, rex } = appOf("local");
    const places: string[] = [];
    followPane(app, "p1", ({ where }) => places.push(where));
    rex.getState().set(rexList(DEVBOX));
    expect(new Set(places)).toEqual(new Set(["C:\\here"]));
  });
});
