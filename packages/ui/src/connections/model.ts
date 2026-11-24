/**
 * What the dialog that adds a connection works with: how an address is read, what a
 * test of one goes through, the profile a machine reached over SSH becomes, and which
 * Linux distributions are listed.
 *
 * Nothing here touches the page, so all of it is tested with plain objects.
 */

import type {
  ListedProfile,
  Profile,
  RexAddress,
  RexHost,
  RexTried,
  SshHost,
} from "../core/types.ts";
import { systemName } from "../rex/model.ts";

/** What can be added. */
export type ConnectionKind = "rex" | "ssh" | "wsl";

/** What is changed, in place of something new. */
export type EditedConnection =
  | { readonly kind: "rex"; readonly label: string; readonly address: string }
  | { readonly kind: "ssh"; readonly profile: Profile };

/** What an address came out as: how the core reads it, or why it cannot. */
export type AddressRead =
  | { readonly ok: true; readonly address: RexAddress }
  | { readonly ok: false; readonly code: string; readonly message: string };

/** What is said of an address under its field. */
export interface AddressWords {
  readonly tone: "idle" | "good" | "warn";
  readonly icon: "key-round" | "globe" | "server" | "triangle-exclamation-point";
  /** The way there, said first. */
  readonly head: string;
  readonly text: string;
}

const LOOPBACK = /^(?:localhost|[\w.-]+\.localhost|127(?:\.\d{1,3}){3}|::1)$/i;

/** Whether nothing between the two ends is encrypted on a network, whatever is done. */
function inTheOpen(address: RexAddress): boolean {
  return address.kind === "http" && !LOOPBACK.test(address.host ?? "");
}

/** A host and a port as they are written together. */
function at(address: RexAddress): string {
  const host = address.host ?? "";
  const named = host.includes(":") ? `[${host}]` : host;
  return address.port == null ? named : `${named}:${address.port}`;
}

/** What is said of an address as it is typed. */
export function addressWords(typed: string, read: AddressRead | null): AddressWords {
  if (typed.trim() === "" || read === null) {
    return {
      tone: "idle",
      icon: "globe",
      head: "",
      text:
        "Paste the address you use with rex -S. Over SSH, Rex uses your keys and " +
        "~/.ssh/config and opens no extra port.",
    };
  }
  const warn = (text: string, head = ""): AddressWords => ({
    tone: "warn",
    icon: "triangle-exclamation-point",
    head,
    text,
  });
  if (!read.ok) {
    if (read.code === "unsupported" && typed.includes("@")) {
      return warn(
        `An account can be named only in an ssh:// address. Try ssh://${typed.trim().replace(/^[a-z]+:\/\//i, "")}.`,
      );
    }
    return warn(
      "That doesn't look like an address. Try ssh://user@host or https://host.tailnet.ts.net.",
    );
  }
  const { address } = read;
  if (address.taken !== null) return warn(`That is the server called “${address.taken}”.`);
  const host = address.host ?? "";
  const good = (icon: AddressWords["icon"], head: string, text: string): AddressWords => ({
    tone: "good",
    icon,
    head,
    text,
  });
  switch (address.kind) {
    case "ssh": {
      const who = address.user == null ? "" : ` as ${address.user}`;
      const port = address.port == null ? "" : `, port ${address.port}`;
      return good(
        "key-round",
        "Over SSH",
        address.path == null
          ? `${who} on ${host}${port}. Signs in with your SSH keys and starts your personal Rex server there if it isn't running.`
          : `${who} on ${host}${port}, socket ${address.path}. Signs in with your SSH keys.`,
      );
    }
    case "https":
      return host.endsWith(".ts.net")
        ? good("globe", "Over Tailscale", ` to ${host}. Your tailnet identity signs you in.`)
        : good("globe", "HTTPS", ` to ${host} on port ${address.port ?? 443}.`);
    case "tailnet":
      return good("globe", "Over Tailscale", ` to ${at(address)}. The tailnet encrypts it.`);
    case "http":
      return inTheOpen(address)
        ? warn(
            ` to ${at(address)}. No one is identified: anyone who can reach this port has full access. Only use it on a network you control. Lume asks before it connects.`,
            "Plain HTTP",
          )
        : good("server", "Plain HTTP", ` to ${at(address)}, on this computer.`);
    case "host":
      return good(
        "globe",
        "",
        address.port == null
          ? "Lume tries HTTPS on 443, then HTTPS on 7297, then HTTP on 7297. Start with ssh:// to go over SSH instead."
          : `Lume tries HTTPS on ${address.port}, then HTTP. Start with ssh:// to go over SSH instead.`,
      );
    case "local":
      return good("server", "This computer", ": the Rex server of your account, while it runs.");
    case "pipe":
    case "unix":
      return good("server", "This computer", ` at ${address.path ?? ""}.`);
  }
}

/** What a server is called when the user calls it nothing: the first part of its host. */
export function defaultName(address: AddressRead | null): string {
  if (address === null || !address.ok) return "";
  const { kind, host } = address.address;
  if (kind === "local") return "Local Rex";
  return host === undefined ? "Rex" : nameOfHost(host);
}

/** The first part of a host's name, or the whole of an address made of numbers. */
export function nameOfHost(host: string): string {
  const name = host.trim();
  return /^[\d.]+$/.test(name) || name.includes(":") ? name : (name.split(".")[0] ?? name);
}

/** Why a server cannot be called this, or nothing. */
export function labelTaken(
  label: string,
  hosts: readonly RexHost[],
  was: string | null,
): string | null {
  const wanted = label.trim().toLowerCase();
  const other = hosts.find(
    (host) => host.written && host.label !== was && host.label.toLowerCase() === wanted,
  );
  return other ? `There is already a server called “${other.label}”` : null;
}

/** One thing a test goes through. */
export interface TestStep {
  readonly text: string;
  readonly state: "wait" | "run" | "ok" | "bad";
  /** Said at the end of the line once it is over. */
  readonly detail: string;
}

/** A test, as far as it has gone. */
export interface ConnectionTest {
  readonly state: "run" | "ok" | "bad";
  readonly steps: readonly TestStep[];
  /** Why it failed. */
  readonly failure: TestFailure | null;
}

/** Why a test failed, and what can be done about it. */
export interface TestFailure {
  readonly code: string;
  /** Said beside the buttons. */
  readonly short: string;
  readonly message: string;
  /** Whether what was tried is of no use as it is, rather than out of reach now. */
  readonly final: boolean;
}

/** The codes of failures that another try would not change. */
const FINAL: readonly string[] = [
  "not_installed",
  "untrusted",
  "unsupported",
  "forbidden",
  "rejected",
  "version_mismatch",
  "bad_request",
];

/** A failure in a few words, by the core's code for it. */
const SHORT: Readonly<Record<string, string>> = {
  not_installed: "Rex isn't installed there",
  unreachable: "It could not be reached",
  timeout: "It did not answer in time",
  confirmation_required: "It would not be encrypted",
  untrusted: "Its certificate is not trusted",
  forbidden: "It does not let this computer in",
  rejected: "It does not let this computer in",
  version_mismatch: "It speaks another version of Rex",
  ssh: "SSH did not sign in",
  auth: "SSH did not sign in",
  host_key: "The machine's key is not known",
  unsupported: "It cannot be used",
};

/** A message as a sentence that has an end. */
function ended(message: string): string {
  const text = message.trim();
  const whole = text.charAt(0).toUpperCase() + text.slice(1);
  return /[.!?…]$/.test(whole) ? whole : `${whole}.`;
}

/** What went wrong in a test, from the core's code and words. */
export function testFailure(code: string, message: string): TestFailure {
  const text =
    code === "confirmation_required"
      ? "The connection would not be encrypted. Testing it sends a greeting and nothing else."
      : code === "auth"
        ? `${ended(message)} A terminal can still be opened, and asks for what is missing.`
        : code === "host_key"
          ? `${ended(message)} Connecting once in a terminal settles it.`
          : ended(message);
  return {
    code,
    short: SHORT[code] ?? "It did not work",
    message: text,
    final: FINAL.includes(code),
  };
}

/** What the test of a Rex server goes through: first whether the machine answers. */
export function rexSteps(address: RexAddress): readonly string[] {
  const host = address.host ?? "";
  switch (address.kind) {
    case "ssh":
      return address.path == null
        ? [`Reach ${host} over SSH`, "Sign in and find rex", "Talk to your Rex server"]
        : [`Reach ${host} over SSH`, "Sign in with your SSH keys", "Talk to the Rex server"];
    case "https":
      return [`Reach ${host}`, "Check its certificate", "Talk to the Rex server"];
    case "host":
      return [`Reach ${host}`, "Find how it answers", "Talk to the Rex server"];
    case "http":
    case "tailnet":
      return [`Reach ${host}`, "Talk to the Rex server"];
    default:
      return ["Find the server", "Talk to it"];
  }
}

/** Which step of a Rex server's test a failure belongs to, by its code. */
export function failedStep(address: RexAddress, code: string): number {
  const last = rexSteps(address).length - 1;
  const middle = Math.min(1, last);
  switch (code) {
    case "ssh":
    case "not_installed":
    case "untrusted":
    case "confirmation_required":
    case "bad_request":
      return middle;
    case "unsupported":
      return address.kind === "ssh" ? last : middle;
    case "unreachable":
    case "timeout":
      // Reached, and lost before a server spoke: while the way was being found, else
      // when it was talked to.
      return address.kind === "ssh" || address.kind === "host" ? middle : last;
    default:
      return last;
  }
}

/** A test that has just begun. */
export function testBegun(steps: readonly string[]): ConnectionTest {
  return {
    state: "run",
    failure: null,
    steps: steps.map((text, index) => ({
      text,
      state: index === 0 ? "run" : "wait",
      detail: "",
    })),
  };
}

/** A test whose steps before `upTo` are over, with what each has to say. */
export function testGoesOn(
  test: ConnectionTest,
  upTo: number,
  details: readonly string[],
): ConnectionTest {
  const over = upTo >= test.steps.length;
  return {
    state: over ? "ok" : "run",
    failure: null,
    steps: test.steps.map((step, index) =>
      index < upTo
        ? { ...step, state: "ok", detail: details[index] ?? step.detail }
        : { ...step, state: index === upTo ? "run" : "wait", detail: "" },
    ),
  };
}

/** A test that failed at a step. Those before it went well. */
export function testFailed(
  test: ConnectionTest,
  index: number,
  failure: TestFailure,
): ConnectionTest {
  return {
    state: "bad",
    failure,
    steps: test.steps.map((step, at) =>
      at < index
        ? { ...step, state: "ok" }
        : at === index
          ? { ...step, state: "bad", detail: "" }
          : { ...step, state: "wait", detail: "" },
    ),
  };
}

/** What is said of a server that answered. */
export function triedWords(tried: RexTried): string {
  return `rex ${tried.version} · ${systemName(tried.platform)}`;
}

/** What is said beside the dialog's buttons of a test. */
export function testWords(test: ConnectionTest | null): string {
  if (test === null) return "Testing is optional";
  if (test.state === "run") return "Testing…";
  return test.state === "ok" ? "Connection works" : (test.failure?.short ?? "It did not work");
}

/** What is typed of a machine reached over SSH. */
export interface SshForm {
  readonly host: string;
  readonly user: string;
  readonly port: string;
  /** Whether a key file is named, and which. */
  readonly withKey: boolean;
  readonly key: string;
  /** Arguments of the profile that the form has no field for, which stay. */
  readonly rest: readonly string[];
}

export const DEFAULT_KEY = "~/.ssh/id_ed25519";

/** An empty form. */
export const EMPTY_SSH: SshForm = {
  host: "",
  user: "",
  port: "22",
  withKey: false,
  key: DEFAULT_KEY,
  rest: [],
};

/** The form a profile is changed in. */
export function sshForm(profile: Profile): SshForm {
  const launcher = profile.launcher;
  if (launcher.type !== "ssh") return EMPTY_SSH;
  const args = launcher.extra_args ?? [];
  const named = args.indexOf("-i");
  const key = named >= 0 ? args[named + 1] : undefined;
  return {
    host: launcher.host,
    user: launcher.user ?? "",
    port: String(launcher.port ?? 22),
    withKey: key !== undefined,
    key: key ?? DEFAULT_KEY,
    rest: key === undefined ? args : [...args.slice(0, named), ...args.slice(named + 2)],
  };
}

/** The machine a form describes, or why it describes none. */
export function sshHost(form: SshForm): SshHost | string {
  const host = form.host.trim();
  const user = form.user.trim();
  if (host === "") return "A host is needed";
  if (/\s/.test(host) || host.startsWith("-")) return "That is not a host's name";
  if (/\s/.test(user) || user.startsWith("-")) return "That is not a user's name";
  const port = form.port.trim() === "" ? 22 : Number(form.port);
  if (!Number.isInteger(port) || port < 1 || port > 65535) return "A port is from 1 to 65535";
  const key = form.key.trim();
  if (form.withKey && key === "") return "A key file is needed";
  return {
    host,
    user: user === "" ? null : user,
    // The client's own choice, which its configuration may make, unless another is named.
    port: port === 22 ? null : port,
    args: [...(form.withKey ? ["-i", key] : []), ...form.rest],
  };
}

/** What the test of a machine reached over SSH goes through. */
export function sshSteps(host: SshHost): readonly string[] {
  return [
    `Reach ${host.host} on port ${host.port ?? 22}`,
    "Sign in with your SSH keys",
    "Open a login shell",
  ];
}

/** A host of the user's OpenSSH configuration, offered to fill the dialog from. */
export interface KnownHost {
  readonly name: string;
  readonly address: string;
  /** Whether there is a server at that address already. */
  readonly added: boolean;
}

/** The hosts of the user's OpenSSH configuration that an `ssh://` address can name. */
export function knownHosts(
  profiles: readonly Profile[],
  hosts: readonly RexHost[],
): readonly KnownHost[] {
  return profiles.flatMap((profile) => {
    const launcher = profile.launcher;
    if (!profile.id.startsWith("auto.ssh.") || launcher.type !== "ssh") return [];
    const name = launcher.host;
    // An address's host is read without regard to case, and the configuration's is not.
    if (!/^[a-z0-9][a-z0-9._-]*$/.test(name)) return [];
    const address = `ssh://${name}`;
    const added = hosts.some((host) => host.id === `rex:${address}`);
    return [{ name, address, added }];
  });
}

/** A profile of the user's own that opens a shell over SSH. */
export function ownSshProfiles(profiles: readonly Profile[]): readonly Profile[] {
  return profiles.filter(
    (profile) =>
      profile.origin === "user" &&
      profile.launcher.type === "ssh" &&
      !profile.id.startsWith("auto."),
  );
}

/** A Linux distribution as the dialog lists it. */
export interface DistroRow {
  readonly id: string;
  readonly name: string;
  /** Its version and whether it runs. */
  readonly line: string;
  readonly listed: boolean;
  /** Where its terminals start. Empty is the user's home there. */
  readonly cwd: string;
}

/** The distributions found on this machine, as they stand. */
export function distroRows(profiles: readonly Profile[]): readonly DistroRow[] {
  return profiles.flatMap((profile) =>
    profile.id.startsWith("auto.wsl.") && profile.launcher.type === "wsl"
      ? [
          {
            id: profile.id,
            name: profile.launcher.distro,
            line: profile.detail ?? "WSL",
            listed: !profile.hidden,
            cwd: profile.cwd ?? "",
          },
        ]
      : [],
  );
}

/** What has to be written for the distributions to stand as `rows` say. */
export function distroChanges(
  before: readonly DistroRow[],
  rows: readonly DistroRow[],
): readonly ListedProfile[] {
  return rows.flatMap((row) => {
    const was = before.find((one) => one.id === row.id);
    const cwd = row.cwd.trim();
    if (was && was.listed === row.listed && was.cwd === cwd) return [];
    return [{ id: row.id, listed: row.listed, cwd: cwd === "" ? null : cwd }];
  });
}

/** How many distributions are listed, in words. */
export function distroWords(rows: readonly DistroRow[]): string {
  const listed = rows.filter((row) => row.listed).length;
  if (rows.length === 0) return "";
  return listed === rows.length
    ? rows.length === 1
      ? "It is listed"
      : "All are listed"
    : listed === 0
      ? "None is listed"
      : `${listed} of ${rows.length} listed`;
}
