import { describe, expect, it } from "vitest";

import {
  CONNECT_ACTIONS,
  connectGroups,
  connectSections,
  hostEntries,
  type ConnectRow,
} from "../src/connect/model.ts";
import type { Profile } from "../src/core/types.ts";
import { rank } from "../src/palette/fuzzy.ts";
import { rexTargets } from "../src/palette/rex.ts";
import { DEVBOX, NAS, rexFailed, rexHost, rexList, UP } from "./rex-fixtures.ts";

const PWSH: Profile = {
  id: "auto.pwsh",
  name: "PowerShell",
  launcher: { type: "shell", program: "pwsh", args: [], login: false },
  hidden: false,
  origin: "generated",
};
const LIST = rexList(DEVBOX, NAS);

const idOf = (row: ConnectRow): string => {
  switch (row.type) {
    case "host":
      return row.entry.host.label;
    case "profile":
      return row.entry.profile.name;
    case "session":
      return row.entry.name;
    case "action":
      return row.action.id;
  }
};

describe("the Rex servers in the Connect palette", () => {
  it("are listed in the order they are given, each with how it stands", () => {
    const entries = hostEntries(LIST, "", "all");
    expect(entries.map(({ host, line, meta }) => [host.label, line, meta])).toEqual([
      ["devbox", "2 sessions", "12 ms"],
      ["home-nas", "Not connected", ""],
    ]);
    expect(hostEntries(null, "", "all")).toEqual([]);
  });

  it("are a filter of their own, and no other kind's", () => {
    expect(hostEntries(LIST, "", "rex")).toHaveLength(2);
    for (const filter of ["ssh", "wsl", "local", "detached"] as const) {
      expect(hostEntries(LIST, "", filter)).toEqual([]);
    }
    expect(connectGroups({ profiles: [PWSH], default: null }, "", "rex")).toEqual([]);
  });

  it("are found by name, by address and by a session of theirs", () => {
    const found = (query: string) => hostEntries(LIST, query, "all").map(({ host }) => host.label);
    expect(found("nas")).toEqual(["home-nas"]);
    expect(hostEntries(LIST, "nas", "all")[0]?.indices).toEqual([5, 6, 7]);
    expect(found("https")).toEqual(["home-nas"]);
    expect(found("ml-train")).toEqual(["devbox"]);
    // Each name is matched on its own: the letters of two sessions are not one name.
    expect(found("acme-train")).toEqual([]);
    expect(hostEntries(LIST, "ml-train", "all")[0]?.indices).toEqual([]);
    expect(found("zzz")).toEqual([]);
  });

  it("stand before the profiles, and the pinned ones first of all", () => {
    const pinned = new Set([NAS.id, PWSH.id]);
    const groups = connectGroups({ profiles: [PWSH], default: null }, "", "all", pinned);
    const sections = connectSections(groups, [], hostEntries(LIST, "", "all", pinned));
    expect(sections.map(({ label, rows }) => [label, rows.map(idOf)])).toEqual([
      ["Pinned", ["home-nas", "PowerShell"]],
      ["Rex servers", ["devbox"]],
      ["Actions", CONNECT_ACTIONS.map((action) => action.id)],
    ]);
    const plain = connectSections(
      connectGroups({ profiles: [PWSH], default: null }, "", "all"),
      [],
      hostEntries(LIST, "", "all"),
    );
    expect(plain.map(({ key }) => key)).toEqual(["rex", "local", "actions"]);
  });
});

describe("a server with as many sessions as there may be, called as much as may be", () => {
  const sessions = Array.from({ length: 1024 }, (_, at) => ({
    id: `s:${at}`,
    label: `${"ab-".repeat(340)}${at}`.slice(0, 1024),
  }));
  const servers = Array.from({ length: 4 }, (_, at) =>
    rexHost(`big${at}`, `server ${at}`, { state: UP, sessions }),
  );
  const list = rexList(...servers);

  it("is typed into without waiting", () => {
    const targets = rexTargets(list, null);
    const queries = ["a", "ab", "abababababab", "b".repeat(64), "ab-".repeat(40), "zzzz"];
    const started = performance.now();
    for (const query of queries) {
      hostEntries(list, query, "all");
      rank(targets, query, (target) => target.text);
    }
    // Eight thousand names for each query. With the square of the text it took minutes.
    expect((performance.now() - started) / queries.length).toBeLessThan(1500);
  });
});

describe("the Rex servers in the command palette", () => {
  it("are listed with their sessions, the one on show said to be so", () => {
    const shown = { ...DEVBOX, on_show: "session:2" };
    const targets = rexTargets(rexList(shown, NAS), shown.id);
    expect(targets.map(({ kind, text, sub, meta }) => [kind, text, sub, meta])).toEqual([
      ["host", "devbox", "2 sessions", "12 ms"],
      ["session", "acme", "devbox", "session"],
      ["session", "ml-train", "devbox", "attached"],
      ["host", "home-nas", "Not connected", ""],
    ]);
    expect(targets.filter(({ on }) => on).map(({ text }) => text)).toEqual(["ml-train"]);
    expect(new Set(targets.map(({ key }) => key)).size).toBe(targets.length);
    expect(rexTargets(null, null)).toEqual([]);
  });

  it("are opened, or given a session when they have none", () => {
    const [host, session] = rexTargets(LIST, null);
    expect([host?.command, host?.args]).toEqual(["lume.rex.open", { host: DEVBOX.id }]);
    expect([session?.command, session?.args]).toEqual([
      "lume.rex.open",
      { host: DEVBOX.id, session: "session:1" },
    ]);
    const bare = rexHost("local", "Local Rex", { state: UP });
    expect(rexTargets(rexList(bare), null)[0]?.command).toBe("lume.rex.new-session");
    // One that is not connected is connected by being opened.
    expect(rexTargets(rexList(NAS), null)[0]?.command).toBe("lume.rex.open");
    const odd = rexHost("?odd", "odd", { state: rexFailed("bad_request", "x") });
    expect(rexTargets(rexList(odd), null)).toEqual([]);
  });
});
