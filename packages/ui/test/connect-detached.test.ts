import { describe, expect, it } from "vitest";

import {
  CONNECT_ACTIONS,
  CONNECT_FILTERS,
  connectFilters,
  connectGroups,
  connectRows,
  detachedEntries,
  detachedName,
  reattachCommand,
  startedAgo,
  stepFilter,
} from "../src/connect/model.ts";
import { createConnectStore } from "../src/connect/store.ts";
import type { DetachedSession, Profile } from "../src/core/types.ts";

const MINUTE = 60_000;
const NOW = 1_800_000_000_000;

function session(extra: Partial<DetachedSession> = {}): DetachedSession {
  return {
    backend: "local",
    session: "00000000000000a1",
    program: "C:\\Windows\\System32\\cmd.exe",
    title: null,
    cwd: null,
    created_unix_ms: NOW - 5 * MINUTE,
    exited: false,
    pid: 100,
    ...extra,
  };
}

const DEV = session({
  session: "00000000000000d1",
  program: "/usr/bin/zsh",
  title: "npm run dev",
  cwd: "/home/mira/site",
  created_unix_ms: NOW - 3 * 60 * MINUTE,
});
const ENDED = session({ session: "00000000000000e1", exited: true, pid: null });
const BUILD = session({
  session: "00000000000000b1",
  program: "/usr/bin/bash",
  title: "  ",
  cwd: "/srv/build",
});

const PROFILE: Profile = {
  id: "auto.pwsh",
  name: "PowerShell",
  launcher: { type: "shell", program: "pwsh.exe", args: [], login: false },
  hidden: false,
  origin: "generated",
};

describe("the detached sessions of the Connect palette", () => {
  it("are called by the title their program gave them, or by the program", () => {
    expect(detachedName(DEV)).toBe("npm run dev");
    expect(detachedName(ENDED)).toBe("cmd.exe");
    // A title of blanks is no title.
    expect(detachedName(BUILD)).toBe("bash");
  });

  it("say where they are, whether they run and when they started", () => {
    const [dev, ended] = detachedEntries([DEV, ENDED], "", "all", NOW, (cwd) =>
      cwd.replace("/home/mira", "~"),
    );
    expect(dev?.line).toBe("~/site · running · started 3h ago");
    expect(ended?.line).toBe("ended · started 5m ago");
  });

  it("count their age in minutes, hours and days", () => {
    expect(startedAgo(NOW - 20_000, NOW)).toBe("started just now");
    expect(startedAgo(NOW + 5_000, NOW)).toBe("started just now");
    expect(startedAgo(NOW - 59 * MINUTE, NOW)).toBe("started 59m ago");
    expect(startedAgo(NOW - 25 * 60 * MINUTE, NOW)).toBe("started 1d ago");
  });

  it("are listed under All and Detached, and not under a kind of profile", () => {
    expect(detachedEntries([DEV], "", "all", NOW)).toHaveLength(1);
    expect(detachedEntries([DEV], "", "detached", NOW)).toHaveLength(1);
    expect(detachedEntries([DEV], "", "local", NOW)).toEqual([]);
    expect(detachedEntries(null, "", "all", NOW)).toEqual([]);
    // Under Detached no profile is listed.
    expect(connectGroups({ profiles: [PROFILE], default: null }, "", "detached")).toEqual([]);
  });

  it("are found by name first, then by program and directory", () => {
    const found = detachedEntries([BUILD, ENDED, DEV], "dev", "all", NOW);
    expect(found.map((entry) => entry.session.session)).toEqual(["00000000000000d1"]);
    expect(found[0]?.indices).toEqual([8, 9, 10]);
    const byDirectory = detachedEntries([BUILD, ENDED, DEV], "srv", "all", NOW);
    expect(byDirectory.map((entry) => entry.session.session)).toEqual(["00000000000000b1"]);
    expect(byDirectory[0]?.indices).toEqual([]);
    const byProgram = detachedEntries([BUILD, ENDED, DEV], "cmd.exe", "detached", NOW);
    expect(byProgram.map((entry) => entry.name)).toEqual(["cmd.exe"]);
  });

  it("come after the profiles and before the actions", () => {
    const groups = connectGroups({ profiles: [PROFILE], default: null }, "", "all");
    const rows = connectRows(groups, detachedEntries([DEV, ENDED], "", "all", NOW));
    expect(
      rows.map((row) =>
        row.type === "profile"
          ? row.entry.profile.id
          : row.type === "session"
            ? row.entry.session.session
            : row.action.id,
      ),
    ).toEqual([
      "auto.pwsh",
      "00000000000000d1",
      "00000000000000e1",
      ...CONNECT_ACTIONS.map((action) => action.id),
    ]);
  });

  it("are a filter of their own while there are any, or while it is the one in use", () => {
    const values = (hasDetached: boolean, current: Parameters<typeof connectFilters>[1]) =>
      connectFilters(hasDetached, current).map((choice) => choice.value);
    expect(values(true, "all")).toEqual(CONNECT_FILTERS.map((choice) => choice.value));
    expect(values(false, "all")).toEqual(["all", "ssh", "wsl", "local"]);
    expect(values(false, "detached")).toContain("detached");
    // Stepping through the filters offered passes over the one that is not.
    expect(stepFilter("local", 1, connectFilters(false, "local"))).toBe("all");
    expect(stepFilter("local", 1, connectFilters(true, "local"))).toBe("detached");
    expect(stepFilter("detached", 1, connectFilters(true, "detached"))).toBe("all");
  });

  it("are shown again in a new tab or beside the pane that has the keyboard", () => {
    expect(reattachCommand(DEV, "tab")).toEqual({
      id: "lume.session.reattach",
      args: { session: "00000000000000d1", backend: "local", place: "tab" },
    });
    expect(reattachCommand(DEV, "split").args.place).toBe("split");
  });

  it("are what the palette opens on when it is asked for them", () => {
    const store = createConnectStore();
    store.getState().open("detached");
    expect(store.getState()).toMatchObject({ shown: true, filter: "detached", opening: 1 });
    store.getState().close();
    store.getState().toggle();
    expect(store.getState()).toMatchObject({ shown: true, filter: "all", opening: 2 });
  });
});
