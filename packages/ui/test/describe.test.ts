import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import type { Profile, ProfileList } from "../src/core/types.ts";
import {
  describePane,
  paneLabel,
  type DescribeContext,
  type PaneFacts,
} from "../src/events/describe.ts";
import { iconChoices, PROGRAM_ICONS, programIcon } from "../src/events/programs.ts";
import type { PaneReport } from "../src/events/store.ts";
import {
  homeShortened,
  isThisMachine,
  lastFolder,
  localPlace,
  parseFileUrl,
  windowsPath,
} from "../src/events/where.ts";
import { PaneLooks } from "../src/panes/looks.ts";
import { DEFAULT_APPEARANCE } from "../src/theme.ts";

const MACHINE = { name: "Relrin", home: "C:\\Users\\Lymevyr" };
const UNIX = { name: "box.local", home: "/home/me" };

function profile(id: string, name: string, launcher: Profile["launcher"], more = {}): Profile {
  return { id, name, launcher, hidden: false, origin: "generated", ...more };
}

const PROFILES: ProfileList = {
  revision: 1,
  default: null,
  notes: [],
  looked: true,
  profiles: [
    profile("auto.wsl.Ubuntu", "Ubuntu", { type: "wsl", distro: "Ubuntu" }),
    profile("auto.ssh.box", "box", { type: "ssh", host: "box" }),
    profile("work", "Work", { type: "ssh", host: "10.0.4.21", user: "mira" }),
  ],
};

const WINDOWS: DescribeContext = {
  machine: MACHINE,
  windows: true,
  profiles: PROFILES,
  icons: new Map(),
};

const NO_FACTS: PaneFacts = { name: null, startCwd: null, launcher: null, profile: null };

function report(more: Partial<PaneReport> = {}): PaneReport {
  return { session: "s", statuses: [], activity: "idle", ...more };
}

const PWSH = {
  type: "shell",
  program: "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
  args: [],
  login: false,
} as const;

describe("reading where a shell says it is", () => {
  it("reads a file URL as the core does", () => {
    expect(parseFileUrl("file://Relrin/C:/Users/Lymevyr")).toEqual({
      host: "Relrin",
      path: "/C:/Users/Lymevyr",
    });
    expect(parseFileUrl("FILE:///home/me/a%20b%C3%A9")).toEqual({
      host: "",
      path: "/home/me/a bé",
    });
    expect(parseFileUrl("file://box")).toBeUndefined();
    expect(parseFileUrl("http://box/x")).toBeUndefined();
    // A percent sign that begins no encoding, and a control character, refuse it.
    expect(parseFileUrl("file://box/100%")).toBeUndefined();
    expect(parseFileUrl("file://box/a%0Ab")).toBeUndefined();
    expect(parseFileUrl("file://box/%FF")).toBeUndefined();
  });

  it("knows this machine by its short name, without case, and by no name", () => {
    expect(isThisMachine("", MACHINE, true)).toBe(true);
    expect(isThisMachine("localhost", MACHINE, true)).toBe(true);
    expect(isThisMachine("relrin.corp.example", MACHINE, true)).toBe(true);
    expect(isThisMachine("other", MACHINE, true)).toBe(false);
    const long = { name: "ABCDEFGHIJKLMNO", home: null };
    expect(isThisMachine("abcdefghijklmnopq", long, true)).toBe(true);
    expect(isThisMachine("abcdefghijklmnopq", long, false)).toBe(false);
    expect(isThisMachine("x", { name: null, home: null }, false)).toBe(false);
  });

  it("writes a drive's path as Windows does, and shortens the home folder", () => {
    expect(windowsPath("/c:/Users/me")).toBe("C:\\Users\\me");
    expect(windowsPath("/C:")).toBe("C:\\");
    expect(windowsPath("/usr/bin")).toBeUndefined();
    expect(homeShortened("c:\\users\\lymevyr\\src", MACHINE.home, true)).toBe("~\\src");
    expect(homeShortened("C:\\Users\\Lymevyr", MACHINE.home, true)).toBe("~");
    expect(homeShortened("C:\\Users\\Lymevyrx", MACHINE.home, true)).toBe("C:\\Users\\Lymevyrx");
    expect(homeShortened("/home/me/src", "/home/me", false)).toBe("~/src");
    expect(homeShortened("/home/Me/src", "/home/me", false)).toBe("/home/Me/src");
    expect(lastFolder("~\\src\\lume")).toBe("lume");
    expect(lastFolder("~")).toBe("~");
    expect(lastFolder("/")).toBe("/");
  });

  it("reads a report from another machine in a local pane as that machine's", () => {
    expect(localPlace("file://devbox/srv/app", MACHINE, true)).toEqual({
      elsewhere: "devbox",
      path: "/srv/app",
    });
    expect(localPlace("file://Relrin/C:/x", MACHINE, true)).toEqual({
      elsewhere: null,
      path: "C:\\x",
    });
    expect(localPlace("file://Relrin/usr", MACHINE, true)).toBeUndefined();
  });
});

describe("what a pane is called and where it is", () => {
  it("names a local shell after the program that has its terminal", () => {
    const at = report({
      launcher: PWSH,
      cwd: "file://Relrin/C:/Users/Lymevyr/src/lume",
    });
    const idle = describePane(at, NO_FACTS, WINDOWS);
    expect(idle).toMatchObject({
      kind: "shell",
      name: "pwsh",
      icon: "terminal",
      where: "~\\src\\lume",
    });
    expect(idle.whereInFull).toBe("C:\\Users\\Lymevyr\\src\\lume");
    expect(paneLabel(idle)).toBe("pwsh · lume");

    const editing = describePane(
      {
        ...at,
        foreground: { pid: 9, name: "nvim.exe" },
        command: { state: "running", command: "nvim" },
      },
      NO_FACTS,
      WINDOWS,
    );
    expect(editing).toMatchObject({ name: "nvim", icon: "editor", running: true });
  });

  it("takes the command line's program when only the shell is seen to run", () => {
    const gitBash = {
      type: "shell",
      program: "C:\\Program Files\\Git\\bin\\bash.exe",
      args: ["--login", "-i"],
      login: false,
    } as const;
    // Under Git for Windows a program loses its parent as it starts, so the shell is
    // all that is seen below the terminal.
    const lost = describePane(
      report({
        launcher: gitBash,
        foreground: { pid: 2, name: "bash" },
        command: { state: "running", command: "vim -i NONE notes.md" },
      }),
      NO_FACTS,
      WINDOWS,
    );
    expect(lost).toMatchObject({ name: "vim", icon: "editor", running: true });
    // A program that is seen is what is named, whatever the command line says.
    const seen = describePane(
      report({
        launcher: gitBash,
        foreground: { pid: 3, name: "less.exe" },
        command: { state: "running", command: "git log" },
      }),
      NO_FACTS,
      WINDOWS,
    );
    expect(seen.name).toBe("less");
    // A shell the pane did not name is known by its name too.
    const unnamed = describePane(
      report({
        launcher: { type: "shell", program: "", args: [], login: false },
        foreground: { pid: 4, name: "pwsh" },
        command: { state: "running", command: "cargo build" },
      }),
      NO_FACTS,
      WINDOWS,
    );
    expect(unnamed.name).toBe("cargo");
    // At the prompt the shell is the name.
    expect(
      describePane(
        report({
          launcher: gitBash,
          foreground: { pid: 2, name: "bash" },
          command: { state: "finished", exit: 0 },
        }),
        NO_FACTS,
        WINDOWS,
      ).name,
    ).toBe("bash");
  });

  it("falls back to where the pane started, and to a plain word", () => {
    const started = describePane(
      report(),
      { ...NO_FACTS, startCwd: "C:\\Users\\Lymevyr" },
      WINDOWS,
    );
    expect(started).toMatchObject({ name: "Terminal", where: "~" });
    expect(paneLabel(started)).toBe("Terminal · ~");
    expect(paneLabel(describePane(report(), NO_FACTS, WINDOWS))).toBe("Terminal");
  });

  it("puts the name the user gave first, and keeps the picture of what runs", () => {
    const named = describePane(
      report({ foreground: { pid: 1, name: "htop" } }),
      { ...NO_FACTS, name: "Watching" },
      WINDOWS,
    );
    expect(named).toMatchObject({ name: "Watching", icon: "monitor" });
  });

  it("names a distribution's pane after its command, or its profile, and says where", () => {
    const facts = { ...NO_FACTS, profile: "auto.wsl.Ubuntu" };
    // The core says what the session runs; the pane itself may not.
    const idle = describePane(
      report({
        profile: "auto.wsl.Ubuntu",
        cwd: "file://Relrin/home/relrin",
        foreground: { pid: 3, name: "wsl" },
      }),
      facts,
      WINDOWS,
    );
    expect(idle).toMatchObject({ kind: "wsl", name: "Ubuntu", where: "/home/relrin" });
    const running = describePane(
      report({
        launcher: { type: "wsl", distro: "Ubuntu" },
        cwd: "file://Relrin/home/relrin",
        command: { state: "running", command: "vim notes.md" },
      }),
      NO_FACTS,
      WINDOWS,
    );
    expect(running).toMatchObject({ name: "vim", icon: "editor", where: "Ubuntu:/home/relrin" });
  });

  it("shows another machine's place with its name, and its title before it says", () => {
    const ssh = { type: "ssh", host: "box" } as const;
    const quiet = describePane(report({ launcher: ssh, title: "me@box: ~" }), NO_FACTS, WINDOWS);
    expect(quiet).toMatchObject({ kind: "ssh", name: "box", icon: "remote", where: "me@box: ~" });
    const there = describePane(
      report({ launcher: ssh, profile: "work", cwd: "file://box/srv/app" }),
      NO_FACTS,
      WINDOWS,
    );
    expect(there).toMatchObject({ name: "Work", where: "box:/srv/app" });
  });

  it("passes over a title that is only the client's path, and names the host instead", () => {
    const box = { type: "ssh", host: "lumebox", user: "lume" } as const;
    const quiet = describePane(
      report({ launcher: box, profile: "work", title: "C:\\WINDOWS\\System32\\OpenSSH\\ssh.exe" }),
      NO_FACTS,
      WINDOWS,
    );
    expect(quiet).toMatchObject({ name: "Work", where: "lume@lumebox" });
    expect(paneLabel(quiet)).toBe("Work · lume@lumebox");
    // Named after the host already, it says nothing more.
    const bare = describePane(
      report({ launcher: { type: "ssh", host: "box" }, title: "/usr/bin/ssh.exe" }),
      NO_FACTS,
      WINDOWS,
    );
    expect(bare).toMatchObject({ name: "box", where: "" });
  });

  it("says how a command ended and what a program reports", () => {
    const failed = describePane(
      report({ command: { state: "finished", exit: 3 } }),
      NO_FACTS,
      WINDOWS,
    );
    expect(failed.failed).toBe(3);
    expect(
      describePane(report({ command: { state: "finished", exit: 0 } }), NO_FACTS, WINDOWS).failed,
    ).toBeNull();
    const busy = describePane(
      report({
        statuses: [
          { state: "done", id: "a", message: "built" },
          { state: "working", id: "b" },
          { state: "blocked", id: "c", kind: "question", message: "Allow?" },
          { state: "idle", id: "d" },
        ],
      }),
      NO_FACTS,
      WINDOWS,
    );
    expect(busy.status).toEqual({ state: "blocked", label: "Waiting on you", tooltip: "Allow?" });
    const done = describePane(
      report({ statuses: [{ state: "done", title: "tests" }] }),
      NO_FACTS,
      WINDOWS,
    );
    expect(done.status).toEqual({ state: "done", label: "Finished", tooltip: "tests" });
    expect(
      describePane(report({ statuses: [{ state: "idle" }] }), NO_FACTS, WINDOWS).status,
    ).toBeNull();
  });

  it("says how a connection ended and offers to connect again", () => {
    const lost = describePane(
      report({
        launcher: { type: "ssh", host: "box" },
        disconnected: { reason: "connection_lost", detail: "Broken pipe" },
        exit: { code: 255 },
        statuses: [{ state: "working" }],
      }),
      NO_FACTS,
      WINDOWS,
    );
    expect(lost.ended).toEqual({
      reason: "Connection lost",
      detail: "Broken pipe",
      again: "reconnect",
    });
    expect(lost.status).toBeNull();
    const remoteExit = describePane(
      report({
        launcher: { type: "ssh", host: "box" },
        disconnected: { reason: "exited" },
        exit: { code: 7 },
      }),
      NO_FACTS,
      WINDOWS,
    );
    expect(remoteExit.ended?.reason).toBe("Ended with code 7");
    const local = describePane(report({ exit: { signal: 9 } }), NO_FACTS, WINDOWS);
    expect(local.ended).toEqual({ reason: "Ended by signal 9", detail: null, again: "restart" });
  });

  it("reads Unix paths on a Unix machine", () => {
    const unix = { ...WINDOWS, machine: UNIX, windows: false };
    const shell = { type: "shell", program: "/bin/zsh", args: [], login: true } as const;
    expect(
      describePane(report({ launcher: shell, cwd: "file://box/home/me/src" }), NO_FACTS, unix),
    ).toMatchObject({ name: "zsh", where: "~/src" });
  });
});

describe("the pictures of programs", () => {
  it("knows programs by name, without folder, .exe or case, and lets the user choose", () => {
    expect(programIcon("C:\\Tools\\LazyGit.EXE", new Map())).toBe("git");
    expect(programIcon("unheard-of", new Map())).toBeUndefined();
    const choices = iconChoices({ "Unheard-Of.exe": "agent", vim: "files", bad: "sparkles", n: 3 });
    expect([...choices]).toEqual([
      ["unheard-of", "agent"],
      ["vim", "files"],
    ]);
    expect(programIcon("vim", choices)).toBe("files");
    const named = describePane(report({ foreground: { pid: 1, name: "unheard-of" } }), NO_FACTS, {
      ...WINDOWS,
      icons: choices,
    });
    expect(named.icon).toBe("agent");
  });

  it("are the ones the configuration can name", () => {
    interface Schema {
      readonly properties: {
        readonly appearance: {
          readonly properties: {
            readonly "program-icons": {
              readonly additionalProperties: {
                readonly oneOf: readonly { readonly const: string }[];
              };
            };
          };
        };
      };
    }
    const schema = JSON.parse(
      readFileSync(new URL("../../../schemas/config.schema.json", import.meta.url), "utf8"),
    ) as Schema;
    const icons = schema.properties.appearance.properties["program-icons"].additionalProperties;
    expect(icons.oneOf.map((one) => one.const)).toEqual([...PROGRAM_ICONS]);
  });
});

describe("a pane's own look", () => {
  const themed: ProfileList["looks"] = {};

  it("is the window's when the profile asks for nothing, and the same object each time", () => {
    const looks = new PaneLooks();
    expect(looks.of(DEFAULT_APPEARANCE, undefined, themed)).toBe(DEFAULT_APPEARANCE);
    const big = profile("big", "Big", PWSH, { font_size: 20 });
    const once = looks.of(DEFAULT_APPEARANCE, big, themed);
    expect(once.fontSizePx).toBe(20);
    expect(looks.of(DEFAULT_APPEARANCE, big, themed)).toBe(once);
    // A theme the core could not find leaves the window's colours.
    const lost = profile("lost", "Lost", PWSH, { theme: "nowhere" });
    expect(looks.of(DEFAULT_APPEARANCE, lost, themed)).toBe(DEFAULT_APPEARANCE);
  });
});
