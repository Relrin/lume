import { describe, expect, it } from "vitest";

import {
  addressWords,
  defaultName,
  distroChanges,
  distroRows,
  distroWords,
  EMPTY_SSH,
  EMPTY_WSL,
  failedStep,
  knownHosts,
  labelTaken,
  nameOfHost,
  ownSshProfiles,
  ownWslProfiles,
  rexSteps,
  sshForm,
  sshHost,
  testBegun,
  testFailed,
  testFailure,
  testGoesOn,
  testWords,
  wslForm,
  wslReady,
  wslShell,
  type AddressRead,
} from "../src/connections/model.ts";
import type { Profile, RexAddress } from "../src/core/types.ts";
import { rexHost } from "./rex-fixtures.ts";

function address(kind: RexAddress["kind"], extra: Partial<RexAddress> = {}): RexAddress {
  return { kind, host: "build.example.com", port: null, taken: null, ...extra };
}

function read(kind: RexAddress["kind"], extra: Partial<RexAddress> = {}): AddressRead {
  return { ok: true, address: address(kind, extra) };
}

function profile(id: string, launcher: Profile["launcher"], extra: Partial<Profile> = {}): Profile {
  return { id, name: id, launcher, hidden: false, origin: "generated", ...extra };
}

describe("an address as it is typed", () => {
  it("is said to lead where it does", () => {
    const words = (typed: string, what: AddressRead | null) => {
      const said = addressWords(typed, what);
      return [said.tone, said.head + said.text];
    };
    expect(words("", null)[0]).toBe("idle");
    expect(
      words("ssh://al@build.example.com:2222", read("ssh", { user: "al", port: 2222 }))[1],
    ).toMatch(/^Over SSH as al on build\.example\.com, port 2222\. .* starts your personal/);
    // A socket that is named is that of a server that runs.
    expect(words("x", read("ssh", { path: "/run/rex.sock" }))[1]).not.toContain("starts");
    expect(words("x", read("https", { port: 8443 }))[1]).toContain("on port 8443");
    expect(words("x", read("https", { host: "a.b.ts.net", port: 443 }))[1]).toMatch(
      /^Over Tailscale to a\.b\.ts\.net/,
    );
    expect(words("x", read("host"))[1]).toContain("HTTPS on 7297");
    expect(words("x", read("host", { port: 9 }))[1]).toContain("HTTPS on 9");
    expect(words("x", read("local"))[0]).toBe("good");
  });

  it("is warned of where nothing would be encrypted, except on this machine", () => {
    expect(addressWords("x", read("http", { host: "192.0.2.1", port: 7297 })).tone).toBe("warn");
    for (const host of ["localhost", "127.0.0.1", "::1"]) {
      expect(addressWords("x", read("http", { host, port: 7297 })).tone).toBe("good");
    }
  });

  it("is said to be wrong, with what to type instead", () => {
    const wrong: AddressRead = { ok: false, code: "bad_request", message: "no" };
    expect(addressWords("ftp://x", wrong).text).toContain("doesn't look like an address");
    const account: AddressRead = { ok: false, code: "unsupported", message: "no" };
    expect(addressWords("al@build", account).text).toContain("Try ssh://al@build.");
    expect(addressWords("https://al@build", account).text).toContain("Try ssh://al@build.");
    expect(addressWords("x", read("host", { taken: "nas" })).text).toContain("“nas”");
  });

  it("gives a name, unless another server has it", () => {
    expect(defaultName(read("ssh"))).toBe("build");
    expect(defaultName(read("local"))).toBe("Local Rex");
    expect(defaultName(null)).toBe("");
    expect(nameOfHost("10.0.0.4")).toBe("10.0.0.4");
    expect(nameOfHost("::1")).toBe("::1");
    const hosts = [
      rexHost("https://nas", "NAS"),
      rexHost("local", "Local Rex", { written: false }),
    ];
    expect(labelTaken(" nas ", hosts, null)).toContain("“NAS”");
    expect(labelTaken("nas", hosts, "NAS")).toBeNull();
    // The server of this machine is called something only until it is written.
    expect(labelTaken("Local Rex", hosts, null)).toBeNull();
  });
});

describe("a test", () => {
  it("goes through its steps and says how it stands", () => {
    const begun = testBegun(rexSteps(address("ssh")));
    expect(begun.steps.map((step) => step.state)).toEqual(["run", "wait", "wait"]);
    const on = testGoesOn(begun, 1, ["12 ms"]);
    expect(on.steps.map((step) => step.state)).toEqual(["ok", "run", "wait"]);
    expect(on.steps[0]?.detail).toBe("12 ms");
    const over = testGoesOn(on, 3, ["12 ms", "", "rex 1"]);
    expect(over.state).toBe("ok");
    expect([null, begun, over].map(testWords)).toEqual([
      "Testing is optional",
      "Testing…",
      "Connection works",
    ]);
    const failed = testFailed(on, 1, testFailure("not_installed", "no rex"));
    expect(failed.steps.map((step) => step.state)).toEqual(["ok", "bad", "wait"]);
    expect(testWords(failed)).toBe("Rex isn't installed there");
  });

  it("puts a failure where it belongs", () => {
    const at = (kind: RexAddress["kind"], code: string) =>
      rexSteps(address(kind))[failedStep(address(kind), code)];
    expect(at("ssh", "not_installed")).toBe("Sign in and find rex");
    expect(at("ssh", "ssh")).toBe("Sign in and find rex");
    expect(at("ssh", "version_mismatch")).toBe("Talk to your Rex server");
    expect(at("https", "untrusted")).toBe("Check its certificate");
    expect(at("https", "unreachable")).toBe("Talk to the Rex server");
    expect(at("host", "confirmation_required")).toBe("Find how it answers");
    expect(at("http", "confirmation_required")).toBe("Talk to the Rex server");
    expect(at("tailnet", "forbidden")).toBe("Talk to the Rex server");
    for (const kind of ["local", "pipe", "unix"] as const) {
      expect(failedStep(address(kind), "protocol_error")).toBe(1);
    }
  });

  it("stops what is of no use from being added, and not what is out of reach", () => {
    const final = (code: string) => testFailure(code, "why").final;
    expect(["not_installed", "untrusted", "version_mismatch"].map(final)).toEqual([
      true,
      true,
      true,
    ]);
    expect(["unreachable", "ssh", "confirmation_required", "timeout"].map(final)).toEqual([
      false,
      false,
      false,
      false,
    ]);
  });
});

describe("a machine reached over SSH", () => {
  it("is read from what is typed", () => {
    expect(sshHost({ ...EMPTY_SSH, host: " build ", user: " al " })).toEqual({
      host: "build",
      user: "al",
      port: null,
      args: [],
    });
    expect(
      sshHost({
        ...EMPTY_SSH,
        host: "build",
        port: "2222",
        withKey: true,
        key: "~/k",
        rest: ["-v"],
      }),
    ).toEqual({ host: "build", user: null, port: 2222, args: ["-i", "~/k", "-v"] });
    for (const wrong of [
      { host: "" },
      { host: "-oX" },
      { host: "a b" },
      { host: "a", user: "-x" },
      { host: "a", port: "0" },
      { host: "a", port: "70000" },
      { host: "a", withKey: true, key: " " },
    ]) {
      expect(typeof sshHost({ ...EMPTY_SSH, ...wrong })).toBe("string");
    }
  });

  it("is read from a profile, and keeps the arguments it has no field for", () => {
    const launcher = {
      type: "ssh",
      host: "build",
      extra_args: ["-A", "-i", "~/k", "-v"],
    } as const;
    const form = sshForm(profile("b", launcher));
    expect(form).toMatchObject({ withKey: true, key: "~/k", port: "22", rest: ["-A", "-v"] });
    expect(sshHost(form)).toMatchObject({ args: ["-i", "~/k", "-A", "-v"] });
    expect(sshForm(profile("b", { type: "ssh", host: "build" })).withKey).toBe(false);
  });

  it("is changed only where the user wrote it", () => {
    const ssh = { type: "ssh", host: "x" } as const;
    const own = ownSshProfiles([
      profile("mine", ssh, { origin: "user" }),
      profile("auto.ssh.x", ssh),
      profile("auto.ssh.y", ssh, { origin: "user" }),
      profile("shell", { type: "shell", program: "", args: [], login: false }, { origin: "user" }),
    ]);
    expect(own.map((one) => one.id)).toEqual(["mine"]);
  });
});

describe("a shell in a Linux distribution", () => {
  it("is read from what is typed", () => {
    expect(wslReady(EMPTY_WSL)).toBe(false);
    expect(wslShell(EMPTY_WSL)).toBe("A distro is needed");
    const form = { distro: " Ubuntu ", user: "", cwd: "" };
    expect(wslReady(form)).toBe(true);
    expect(wslShell(form)).toEqual({ distro: "Ubuntu", user: null, cwd: null });
    expect(wslShell({ distro: "Ubuntu", user: " me ", cwd: " ~/code " })).toEqual({
      distro: "Ubuntu",
      user: "me",
      cwd: "~/code",
    });
    for (const wrong of [
      { distro: "--exec", user: "", cwd: "" },
      { distro: "Ubuntu 22", user: "", cwd: "" },
      { distro: "Ubuntu", user: "-r", cwd: "" },
      { distro: "Ubuntu", user: "", cwd: "~/a" },
    ]) {
      expect(wslReady(wrong), JSON.stringify(wrong)).toBe(false);
    }
  });

  it("is read from a profile, and only the user’s own are changed", () => {
    const own = profile(
      "work",
      { type: "wsl", distro: "Ubuntu", user: "me" },
      { origin: "user", cwd: "~/code" },
    );
    expect(wslForm(own)).toEqual({ distro: "Ubuntu", user: "me", cwd: "~/code" });
    expect(wslForm(profile("ssh", { type: "ssh", host: "x" }))).toEqual(EMPTY_WSL);
    const mine = ownWslProfiles([
      own,
      profile("auto.wsl.Ubuntu", { type: "wsl", distro: "Ubuntu" }),
      profile("auto.wsl.Debian", { type: "wsl", distro: "Debian" }, { origin: "user" }),
      profile("box", { type: "ssh", host: "x" }, { origin: "user" }),
    ]);
    expect(mine.map((one) => one.id)).toEqual(["work"]);
  });
});

describe("what there is on this machine", () => {
  it("offers the hosts of the SSH configuration an address can name", () => {
    const hosts = [rexHost("ssh://build", "build")];
    const offered = knownHosts(
      [
        profile("auto.ssh.build", { type: "ssh", host: "build" }),
        profile("auto.ssh.nas", { type: "ssh", host: "nas" }),
        // An address is read without regard to case, so it would name another.
        profile("auto.ssh.Big", { type: "ssh", host: "Big" }),
        profile("mine", { type: "ssh", host: "mine" }, { origin: "user" }),
      ],
      hosts,
    );
    expect(offered).toEqual([
      { name: "build", address: "ssh://build", added: true },
      { name: "nas", address: "ssh://nas", added: false },
    ]);
  });

  it("lists the distributions, and what has to be written for them to stand so", () => {
    const before = distroRows([
      profile("auto.wsl.Ubuntu", { type: "wsl", distro: "Ubuntu" }, { cwd: "~/code" }),
      profile("auto.wsl.Debian", { type: "wsl", distro: "Debian" }, { hidden: true }),
      profile("own", { type: "wsl", distro: "Ubuntu" }, { origin: "user" }),
    ]);
    expect(before.map((row) => [row.name, row.listed, row.cwd])).toEqual([
      ["Ubuntu", true, "~/code"],
      ["Debian", false, ""],
    ]);
    expect(distroWords(before)).toBe("1 of 2 listed");
    expect(distroChanges(before, before)).toEqual([]);
    const after = before.map((row) => ({ ...row, listed: true, cwd: " " }));
    expect(distroChanges(before, after)).toEqual([
      { id: "auto.wsl.Ubuntu", listed: true, cwd: null },
      { id: "auto.wsl.Debian", listed: true, cwd: null },
    ]);
    expect(distroWords(after)).toBe("All are listed");
    expect(distroWords([])).toBe("");
  });
});
