import { describe, expect, it } from "vitest";

import {
  CONNECT_ACTIONS,
  connectGroups,
  connectRows,
  connectSections,
  describeRun,
  kindOf,
  lineOf,
  openCommand,
  sourceOf,
  stepFilter,
} from "../src/connect/model.ts";
import { CONNECT_GROUP_MAX, NEW_TAB_MENU_MAX } from "../src/connect/tuning.ts";
import type { Profile, ProfileList } from "../src/core/types.ts";
import { newTabWithItems } from "../src/shell/AppMenu.tsx";

function shell(id: string, name: string, program: string, extra: Partial<Profile> = {}): Profile {
  return {
    id,
    name,
    launcher: { type: "shell", program, args: [], login: false },
    hidden: false,
    origin: "generated",
    ...extra,
  };
}

const PWSH = shell("auto.pwsh", "PowerShell", "C:\\Program Files\\PowerShell\\7\\pwsh.exe");
const CMD = shell("auto.cmd", "Command Prompt", "C:\\Windows\\System32\\cmd.exe", {
  hidden: true,
});
const NU: Profile = {
  id: "nu",
  name: "Nushell",
  launcher: { type: "shell", program: "nu", args: ["--login", "-e", "say hi"], login: false },
  cwd: "~/src",
  hidden: false,
  origin: "user",
};
const UBUNTU: Profile = {
  id: "auto.wsl.Ubuntu",
  name: "Ubuntu",
  launcher: { type: "wsl", distro: "Ubuntu" },
  hidden: false,
  origin: "generated",
  detail: "WSL 2 · running",
};
const STAGING: Profile = {
  id: "auto.ssh.dev-staging",
  name: "dev-staging",
  launcher: { type: "ssh", host: "dev-staging" },
  hidden: false,
  origin: "generated",
  detail: "from ~/.ssh/config",
};
const BOX: Profile = {
  id: "box",
  name: "Build box",
  launcher: {
    type: "ssh",
    host: "10.0.4.21",
    user: "mira",
    port: 2222,
    extra_args: ["-F", "C:\\my config"],
  },
  hidden: false,
  origin: "user",
};

const LIST: ProfileList = {
  revision: 3,
  // The user's own come first, as the core lists them.
  profiles: [NU, BOX, PWSH, CMD, UBUNTU, STAGING],
  default: "auto.pwsh",
  notes: [],
  looked: true,
};

describe("what the Connect palette lists", () => {
  it("groups the profiles by where their terminal is, leaving the hidden ones out", () => {
    const groups = connectGroups(LIST, "", "all");
    expect(groups.map((group) => group.label)).toEqual([
      "Local shells",
      "WSL distros",
      "SSH hosts",
    ]);
    expect(groups.map((group) => group.entries.map((entry) => entry.profile.id))).toEqual([
      ["nu", "auto.pwsh"],
      ["auto.wsl.Ubuntu"],
      ["box", "auto.ssh.dev-staging"],
    ]);
    // The default is marked, and only the default.
    const marked = groups.flatMap((group) => group.entries).filter((entry) => entry.isDefault);
    expect(marked.map((entry) => entry.profile.id)).toEqual(["auto.pwsh"]);
  });

  it("lists the pinned profiles first, in a group of their own", () => {
    const groups = connectGroups(LIST, "", "all", new Set(["auto.ssh.dev-staging", "nu"]));
    expect(groups.map((group) => group.kind)).toEqual(["pinned", "local", "wsl", "ssh"]);
    expect(groups[0]?.entries.map((entry) => entry.profile.id)).toEqual([
      "nu",
      "auto.ssh.dev-staging",
    ]);
    expect(groups[0]?.entries.every((entry) => entry.pinned)).toBe(true);
    // A pinned profile is not listed twice.
    const ids = groups.flatMap((group) => group.entries.map((entry) => entry.profile.id));
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("leaves out a kind whose only profile is pinned", () => {
    const groups = connectGroups(LIST, "", "all", new Set(["auto.wsl.Ubuntu"]));
    expect(groups.map((group) => group.label)).toEqual(["Pinned", "Local shells", "SSH hosts"]);
  });

  it("lists the user's own profiles first in each group, whatever order they came in", () => {
    const shuffled = { ...LIST, profiles: [STAGING, PWSH, UBUNTU, BOX, NU] };
    const groups = connectGroups(shuffled, "", "all");
    expect(groups[0]!.entries.map((entry) => entry.profile.id)).toEqual(["nu", "auto.pwsh"]);
    expect(groups[2]!.entries.map((entry) => entry.profile.id)).toEqual([
      "box",
      "auto.ssh.dev-staging",
    ]);
  });

  it("shows one kind of profile under a filter", () => {
    expect(connectGroups(LIST, "", "wsl").map((group) => group.kind)).toEqual(["wsl"]);
    expect(connectGroups(LIST, "", "ssh")[0]!.entries).toHaveLength(2);
    expect(connectGroups(LIST, "", "local")[0]!.entries.map((entry) => entry.profile.id)).toEqual([
      "nu",
      "auto.pwsh",
    ]);
  });

  it("ranks by name and marks the letters that matched, then finds the rest by what they run", () => {
    const groups = connectGroups(LIST, "dev", "all");
    expect(groups.map((group) => group.kind)).toEqual(["ssh"]);
    expect(groups[0]!.entries[0]!.profile.id).toBe("auto.ssh.dev-staging");
    expect(groups[0]!.entries[0]!.indices).toEqual([0, 1, 2]);
    // A profile is found by its address, with nothing of its name to mark.
    const byAddress = connectGroups(LIST, "10.0.4", "all");
    expect(byAddress[0]!.entries.map((entry) => entry.profile.id)).toEqual(["box"]);
    expect(byAddress[0]!.entries[0]!.indices).toEqual([]);
    // By its id too.
    const byId = connectGroups(LIST, "auto.wsl", "all").find((group) => group.kind === "wsl");
    expect(byId?.entries.map((entry) => entry.profile.id)).toEqual(["auto.wsl.Ubuntu"]);
    // A query that matches nothing lists nothing; the hidden profile is never found.
    expect(connectGroups(LIST, "zzz", "all")).toEqual([]);
    expect(connectGroups(LIST, "Command", "all")).toEqual([]);
  });

  it("lists nothing before the core has said what there is", () => {
    expect(connectGroups(null, "", "all")).toEqual([]);
    expect(connectRows([])).toEqual(CONNECT_ACTIONS.map((action) => ({ type: "action", action })));
  });

  it("keeps a group to its limit", () => {
    const many = Array.from({ length: CONNECT_GROUP_MAX + 5 }, (_, index) =>
      shell(`s${index}`, `Shell ${index}`, "sh"),
    );
    const of = (pinned?: ReadonlySet<string>) =>
      connectSections(connectGroups({ profiles: many, default: null }, "", "all", pinned));
    // All of them are counted, and as many as are read are listed.
    expect(connectGroups({ profiles: many, default: null }, "", "all")[0]!.entries).toHaveLength(
      many.length,
    );
    const [group, actions] = of();
    expect([group?.rows.length, group?.more]).toEqual([CONNECT_GROUP_MAX, 5]);
    expect(actions?.more).toBe(0);
    // What is pinned is listed, wherever its group had it.
    const [pinned, rest] = of(new Set([many.at(-1)!.id]));
    expect([pinned?.key, pinned?.rows.length, pinned?.more]).toEqual(["pinned", 1, 0]);
    expect(rest?.more).toBe(4);
  });

  it("moves through the profiles, then the actions", () => {
    const rows = connectRows(connectGroups(LIST, "", "all"));
    expect(
      rows.map((row) =>
        row.type === "profile"
          ? row.entry.profile.id
          : row.type === "action"
            ? row.action.id
            : row.type,
      ),
    ).toEqual([
      "nu",
      "auto.pwsh",
      "auto.wsl.Ubuntu",
      "box",
      "auto.ssh.dev-staging",
      "add",
      "config",
      "refresh",
    ]);
    expect(CONNECT_ACTIONS.map((action) => action.command)).toEqual([
      "lume.connection.add",
      "lume.config.open",
      "lume.profile.refresh",
    ]);
  });

  it("steps through the filters in the order they are shown, round and round", () => {
    expect(stepFilter("all", 1)).toBe("rex");
    expect(stepFilter("rex", 1)).toBe("ssh");
    expect(stepFilter("ssh", 1)).toBe("wsl");
    expect(stepFilter("local", 1)).toBe("detached");
    expect(stepFilter("detached", 1)).toBe("all");
    expect(stepFilter("all", -1)).toBe("detached");
  });
});

describe("what the Connect palette says of a profile", () => {
  it("says what starting it runs, as it would be typed", () => {
    expect(describeRun(PWSH)).toBe('"C:\\Program Files\\PowerShell\\7\\pwsh.exe"');
    expect(describeRun(NU)).toBe('nu --login -e "say hi"');
    expect(describeRun(shell("x", "Mine", " "))).toBe("your own shell");
    expect(describeRun(UBUNTU)).toBe("wsl.exe -d Ubuntu");
    expect(describeRun({ launcher: { type: "wsl", distro: "Debian", user: "root" } })).toBe(
      "wsl.exe -d Debian -u root",
    );
    expect(describeRun(STAGING)).toBe("ssh dev-staging");
    expect(describeRun(BOX)).toBe('ssh -F "C:\\my config" -p 2222 mira@10.0.4.21');
    expect(
      describeRun({ launcher: { type: "ssh", host: "h", client: "C:\\Tools\\ssh.exe" } }),
    ).toBe("C:\\Tools\\ssh.exe h");
  });

  it("says where the terminal goes under its name", () => {
    expect(lineOf(BOX)).toBe("mira@10.0.4.21:2222");
    expect(lineOf(STAGING)).toBe("dev-staging");
    expect(lineOf(UBUNTU)).toBe("WSL 2 · running");
    expect(lineOf({ ...UBUNTU, detail: undefined } as Profile)).toBe("WSL · Ubuntu");
    expect(lineOf(PWSH)).toBe("pwsh.exe");
    expect(lineOf(shell("x", "Mine", ""))).toBe("Your own shell");
  });

  it("says where it comes from", () => {
    expect(sourceOf(NU)).toBe("Your profile, in config.toml");
    expect(sourceOf(STAGING)).toBe("A host of your SSH config");
    expect(sourceOf(UBUNTU)).toBe("A Linux distribution on this machine");
    expect(sourceOf(PWSH)).toBe("A shell found on this machine");
    expect(kindOf(BOX)).toBe("ssh");
  });

  it("opens in a new tab, or beside the pane that has the keyboard if there is one", () => {
    expect(openCommand("box", "tab", true)).toEqual({
      id: "lume.terminal.new-with-profile",
      args: { profile: "box" },
    });
    expect(openCommand("box", "split", true)).toEqual({
      id: "lume.pane.split-right",
      args: { profile: "box" },
    });
    expect(openCommand("box", "split", false)).toEqual({
      id: "lume.tab.new",
      args: { profile: "box" },
    });
  });
});

describe("the menu of New tab with", () => {
  const opened: (string | null)[] = [];
  const open = (profile: string | null) => opened.push(profile);
  let mores = 0;
  const more = () => (mores += 1);

  it("lists the default terminal, then the others there are to list, then more", () => {
    const items = newTabWithItems(LIST, open, more, "Ctrl Shift L");
    expect(items.map((item) => [item.label, item.hint, item.separated ?? false])).toEqual([
      ["Default terminal", "PowerShell", false],
      ["Nushell", "Local", true],
      ["Build box", "SSH", false],
      ["Ubuntu", "WSL", false],
      ["dev-staging", "SSH", false],
      ["More…", "Ctrl Shift L", true],
    ]);
    // Every terminal has a picture of its kind; the way to the rest has none.
    expect(items.slice(0, -1).every((item) => item.icon !== undefined)).toBe(true);
    expect(items.at(-1)!.icon).toBeUndefined();
    expect(items.some((item) => item.checked !== undefined)).toBe(false);
    items[0]!.choose();
    items[2]!.choose();
    items.at(-1)!.choose();
    expect(opened).toEqual([null, "box"]);
    expect(mores).toBe(1);
  });

  it("names the system's shell when no profile is the default, and keeps to its limit", () => {
    const many = Array.from({ length: NEW_TAB_MENU_MAX + 4 }, (_, index) =>
      shell(`s${index}`, `Shell ${index}`, "sh"),
    );
    const items = newTabWithItems({ ...LIST, profiles: many, default: null }, open, more);
    expect(items[0]).toMatchObject({ key: "default", hint: "System default" });
    // The default, the most others there may be, and the way to the rest.
    expect(items).toHaveLength(NEW_TAB_MENU_MAX + 2);
    expect(items.at(-1)!.hint).toBeUndefined();
    expect(newTabWithItems(null, open, more).map((item) => item.key)).toEqual(["default", "more"]);
  });
});
