import { describe, expect, it } from "vitest";

import type { WorkspaceList } from "../src/core/types.ts";
import {
  browsedHost,
  creatable,
  firstRow,
  listedRows,
  selectorActs,
  selectorHosts,
  selectorLabel,
  selectorRows,
  THIS_PC,
  type SelectorHost,
  type SelectorSources,
} from "../src/shell/host-selector.ts";
import { DEVBOX, NAS, rexFailed, rexHost, rexList, UP } from "./rex-fixtures.ts";

const WORKSPACES: WorkspaceList = {
  active: "lume",
  workspaces: [
    { name: "acme", root: null },
    { name: "lume", root: "D:\\Code\\lume" },
  ],
};
const OWN: SelectorSources = {
  workspaces: WORKSPACES,
  servers: rexList(DEVBOX, NAS),
  backend: null,
};
/** The server's session called "acme" is on show, which a workspace is called too. */
const SERVED: SelectorSources = {
  workspaces: { ...WORKSPACES, active: "acme" },
  servers: rexList({ ...DEVBOX, on_show: "session:1" }, NAS),
  backend: DEVBOX.id,
};

const every = () => true;

function host(sources: SelectorSources, key: string): SelectorHost {
  const found = selectorHosts(sources, "").find((listed) => listed.key === key);
  if (!found) throw new Error(`there is no host ${key}`);
  return found;
}

describe("the hosts of the selector", () => {
  it("are this machine, then the servers, each with a line about it", () => {
    expect(selectorHosts(OWN, "").map(({ name, line, link }) => [name, line, link])).toEqual([
      ["This PC", "2 workspaces", "up"],
      ["devbox", "Rex · 12 ms · 2 sessions", "up"],
      ["home-nas", "Rex · Not connected", "down"],
    ]);
    const bare = { workspaces: null, servers: null, backend: null };
    expect(selectorHosts(bare, "").map(({ line }) => line)).toEqual([""]);
  });

  it("are found by their name and by what they hold", () => {
    const found = (query: string) => selectorHosts(OWN, query).map(({ key }) => key);
    expect(found("NAS")).toEqual([NAS.id]);
    expect(found("ml-")).toEqual([DEVBOX.id]);
    expect(found("acme")).toEqual([THIS_PC, DEVBOX.id]);
    expect(found("zzz")).toEqual([]);
  });

  it("show first what is on show, then what is looked at", () => {
    expect(browsedHost(OWN, "", null)?.key).toBe(THIS_PC);
    expect(browsedHost(SERVED, "", null)?.key).toBe(DEVBOX.id);
    expect(browsedHost(OWN, "", NAS.id)?.key).toBe(NAS.id);
    // A host that is not listed any more gives way to one that is.
    expect(browsedHost(OWN, "ml-", THIS_PC)?.key).toBe(DEVBOX.id);
    // With nothing listed, what was typed is made where the user was looking.
    expect(browsedHost(OWN, "zzz", DEVBOX.id)?.key).toBe(DEVBOX.id);
    expect(browsedHost(OWN, "", "rex:gone")?.key).toBe(THIS_PC);
  });
});

describe("what a host of the selector holds", () => {
  it("is the workspaces of this machine, the one on show said to be so", () => {
    const rows = selectorRows(OWN, host(OWN, THIS_PC), "");
    expect(rows.map(({ name, hint, on }) => [name, hint, on])).toEqual([
      ["acme", "", false],
      ["lume", "lume", true],
    ]);
    // Only the one on show is renamed, and only another is deleted.
    expect(rows.map(({ renames, takes }) => [renames, takes])).toEqual([
      [false, "delete"],
      [true, null],
    ]);
    expect(firstRow(selectorRows(OWN, host(OWN, THIS_PC), ""))).toBe(1);
  });

  it("says no workspace is on show while a session of its name is", () => {
    const rows = selectorRows(SERVED, host(SERVED, THIS_PC), "");
    expect(rows.map(({ on }) => on)).toEqual([false, false]);
    expect(rows.map(({ renames }) => renames)).toEqual([false, false]);
    const sessions = selectorRows(SERVED, host(SERVED, DEVBOX.id), "");
    expect(sessions.map(({ name, on }) => [name, on])).toEqual([
      ["acme", true],
      ["ml-train", false],
    ]);
    expect(sessions.every(({ renames, takes }) => renames && takes === "end")).toBe(true);
  });

  it("lists so many of a server's sessions, the one on show among them", () => {
    const sessions = Array.from({ length: 1024 }, (_, at) => ({ id: `s:${at}`, label: `s${at}` }));
    const many = { ...DEVBOX, sessions, on_show: "s:900" };
    const sources: SelectorSources = { ...SERVED, servers: rexList(many, NAS) };
    const rows = selectorRows(sources, host(sources, many.id), "");
    const listed = listedRows(rows);
    expect([listed.rows.length, listed.more]).toEqual([200, 824]);
    expect(listed.rows[firstRow(listed.rows)]?.name).toBe("s900");
    // One that is not listed is found by its name, and there already.
    const found = listedRows(selectorRows(sources, host(sources, many.id), "s1023"));
    expect(found.rows.map(({ name }) => name)).toEqual(["s1023"]);
    expect(creatable(sources, host(sources, many.id), "s1023")).toBeNull();
  });

  it("is narrowed by what is typed, unless it is the host's own name", () => {
    const names = (query: string) =>
      selectorRows(OWN, host(OWN, DEVBOX.id), query).map(({ name }) => name);
    expect(names("train")).toEqual(["ml-train"]);
    expect(names("devbox")).toEqual(["acme", "ml-train"]);
    expect(names("zzz")).toEqual([]);
  });

  it("is nothing while a server is not connected", () => {
    expect(selectorRows(OWN, host(OWN, NAS.id), "")).toEqual([]);
    // Also while its connection is being made again: nothing can be opened.
    const lost = rexList({ ...DEVBOX, state: { kind: "waiting", error: "x" } });
    const sources = { ...OWN, servers: lost };
    expect(selectorRows(sources, host(sources, DEVBOX.id), "")).toEqual([]);
  });
});

describe("what can be done with a host of the selector", () => {
  const labels = (sources: SelectorSources, key: string, query = "", known = every) =>
    selectorActs(sources, host(sources, key), query, known).map(({ label }) => label);

  it("is what the application can do with workspaces", () => {
    expect(labels(OWN, THIS_PC)).toEqual([
      "Open folder…",
      "New workspace…",
      "Set workspace folder…",
    ]);
    expect(labels(OWN, THIS_PC, "", () => false)).toEqual([]);
    // A workspace's folder is set while one is on show.
    expect(labels(SERVED, THIS_PC)).toEqual(["Open folder…", "New workspace…"]);
  });

  it("is closing the current workspace when it is open beside the one with the windows", () => {
    const both: SelectorSources = {
      ...OWN,
      workspaces: {
        active: "acme",
        workspaces: [
          { name: "acme", root: null, open: true },
          { name: "lume", root: "D:\\Code\\lume", open: true, holds: true },
          { name: "site", root: null },
        ],
      },
    };
    expect(labels(both, THIS_PC)).toContain("Close workspace");
    // The one with the windows is not closed on its own.
    const home = { ...both, workspaces: { ...both.workspaces!, active: "lume" } };
    expect(labels(home, THIS_PC)).not.toContain("Close workspace");
    // An open workspace says so and is not deleted; one that is not open is.
    const rows = selectorRows(home, host(home, THIS_PC), "");
    expect(rows.map(({ name, hint, takes }) => [name, hint, takes])).toEqual([
      ["acme", "open", null],
      ["lume", "lume", null],
      ["site", "", "delete"],
    ]);
  });

  it("is making a session, and letting go of the server", () => {
    expect(labels(OWN, DEVBOX.id)).toEqual(["New session on devbox", "Disconnect"]);
    const edit = "Edit connection…";
    expect(labels(OWN, NAS.id)).toEqual(["Connect", edit]);
    // The server of this machine is nobody's to change until it is written.
    const here = { ...NAS, written: false };
    expect(labels({ ...OWN, servers: rexList(here) }, here.id)).toEqual(["Connect"]);
    const state = (state: typeof UP | ReturnType<typeof rexFailed> | { kind: "connecting" }) => {
      const sources = { ...OWN, servers: rexList({ ...NAS, state }) };
      return labels(sources, NAS.id);
    };
    expect(state({ kind: "connecting" })).toEqual(["Cancel"]);
    expect(state(rexFailed("unreachable", "x"))).toEqual(["Retry", edit]);
    expect(state(rexFailed("confirmation_required", "x"))).toEqual([
      "Connect without encryption",
      edit,
    ]);
    const odd = rexHost("?odd", "odd", { state: rexFailed("bad_request", "x") });
    const sources = { ...OWN, servers: rexList(odd) };
    expect(labels(sources, odd.id)).toEqual([edit]);
  });

  it("is making what was typed, where nothing is called so", () => {
    expect(creatable(OWN, host(OWN, DEVBOX.id), " scratch ")).toBe("scratch");
    expect(creatable(OWN, host(OWN, DEVBOX.id), "ACME")).toBeNull();
    expect(creatable(OWN, host(OWN, THIS_PC), "acme")).toBeNull();
    expect(creatable(OWN, host(OWN, THIS_PC), " ")).toBeNull();
    // Nothing is made on a server that is not connected.
    expect(creatable(OWN, host(OWN, NAS.id), "scratch")).toBeNull();
    expect(labels(OWN, DEVBOX.id, "scratch")[0]).toBe("Create “scratch” on devbox");
  });
});

describe("the button of the selector", () => {
  it("names the host and what is on show, and how near it is", () => {
    expect(selectorLabel("lume", null, undefined, true)).toEqual({
      host: "This PC",
      name: "lume",
      meta: "",
      link: "up",
      said: "Session host connected",
    });
    expect(selectorLabel("lume", null, undefined, false).link).toBe("down");
    expect(selectorLabel("acme", DEVBOX.id, DEVBOX, true)).toMatchObject({
      host: "devbox",
      meta: "12 ms",
      link: "up",
    });
    // The session host of this machine has no part in a server's layout.
    expect(selectorLabel("acme", DEVBOX.id, DEVBOX, false).link).toBe("up");
    const lost = { ...DEVBOX, state: { kind: "waiting", error: "x" } as const };
    // A connection that is being made again is shown by the dot, and said of it.
    expect(selectorLabel("acme", DEVBOX.id, lost, true)).toMatchObject({
      meta: "",
      link: "busy",
      said: "Reconnecting",
    });
    const gone = { ...DEVBOX, state: rexFailed("version_mismatch", "x") };
    expect(selectorLabel("acme", DEVBOX.id, gone, true)).toMatchObject({
      meta: "offline",
      link: "down",
    });
    expect(selectorLabel("acme", "rex:unknown", undefined, true).host).toBe("Rex");
  });
});
