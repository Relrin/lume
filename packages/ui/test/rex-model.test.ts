import { describe, expect, it } from "vitest";

import {
  canDisconnect,
  firstMarked,
  hostLine,
  hostOnShow,
  linkOf,
  nearness,
  next,
  reason,
  rexHostNamed,
  sessionName,
  sessionOnShow,
  standing,
  systemName,
  UNENCRYPTED,
  usable,
} from "../src/rex/model.ts";
import { createRexStore } from "../src/rex/store.ts";
import { DEVBOX, NAS, rexFailed, rexHost, rexList, UP } from "./rex-fixtures.ts";

describe("the Rex servers a page holds", () => {
  it("takes a list only when it is newer than the one held", () => {
    const store = createRexStore();
    expect(store.getState().list).toBeNull();
    expect(store.getState().set({ ...rexList(NAS), revision: 2 })).toBe(true);
    expect(store.getState().set({ ...rexList(), revision: 2 })).toBe(false);
    expect(store.getState().set({ ...rexList(), revision: 1 })).toBe(false);
    expect(store.getState().list?.hosts).toEqual([NAS]);
    expect(store.getState().set({ ...rexList(), revision: 3 })).toBe(true);
    expect(store.getState().list?.hosts).toEqual([]);
  });
});

describe("which Rex server is meant", () => {
  it("is found by its id, its label whatever the case, or its address", () => {
    const list = rexList(DEVBOX, NAS);
    expect(rexHostNamed(list, NAS.id)).toBe(NAS.id);
    expect(rexHostNamed(list, "DevBox")).toBe(DEVBOX.id);
    expect(rexHostNamed(list, "https://nas")).toBe(NAS.id);
    expect(rexHostNamed(list, "nas")).toBeNull();
    expect(rexHostNamed(null, "devbox")).toBeNull();
  });
});

describe("what is said of a Rex server", () => {
  const codes = [
    "unreachable",
    "confirmation_required",
    "untrusted",
    "forbidden",
    "rejected",
    "version_mismatch",
    "ssh",
    "unsupported",
    "bad_request",
    "protocol_error",
  ];

  it("has words for how it stands", () => {
    expect(standing(DEVBOX)).toBe("2 sessions");
    expect(standing(rexHost("a", "a", { state: UP, sessions: [{ id: "1", label: "" }] }))).toBe(
      "1 session",
    );
    expect(standing(NAS)).toBe("Not connected");
    expect(standing(rexHost("a", "a", { state: { kind: "absent" } }))).toBe("Not running");
    expect(standing(rexHost("a", "a", { state: { kind: "connecting" } }))).toBe("Connecting…");
    // That a connection is being made again is shown, not said.
    expect(standing({ ...DEVBOX, state: { kind: "waiting", error: "x" } })).toBe("2 sessions");
    for (const code of codes) {
      const said = standing(rexHost("a", "a", { state: rexFailed(code, "x") }));
      expect(said).not.toBe("");
      expect(said).not.toBe(code);
    }
    expect(standing(rexHost("a", "a", { state: rexFailed("something new", "x") }))).toBe("Failed");
  });

  it("says how near it is, or that it is not there", () => {
    expect(nearness(DEVBOX)).toBe("12 ms");
    expect(nearness({ ...DEVBOX, latency_ms: 0 })).toBe("<1 ms");
    expect(nearness({ ...DEVBOX, latency_ms: null })).toBe("");
    // How near this machine is says nothing, whichever way it is reached.
    expect(nearness({ ...DEVBOX, local: true })).toBe("");
    const here = [
      "npipe:////./pipe/rex",
      "unix:///run/rex.sock",
      "localhost",
      "http://localhost:7297",
      "http://127.0.0.1:7297/",
      "https://[::1]:443",
      "ssh://mira@localhost:2222",
      "http://ubuntu.localhost",
    ];
    for (const address of here)
      expect([address, nearness({ ...DEVBOX, address })]).toEqual([address, ""]);
    for (const address of ["http://localhost.example", "ssh://127.example", "devbox"]) {
      expect([address, nearness({ ...DEVBOX, address })]).toEqual([address, "12 ms"]);
    }
    expect(nearness(NAS)).toBe("");
    expect(nearness({ ...NAS, state: rexFailed("unreachable", "x") })).toBe("offline");
    // One that waits for the user's word may well be there.
    expect(nearness({ ...NAS, state: rexFailed("confirmation_required", "x") })).toBe("");
    expect(hostLine(DEVBOX)).toBe("Rex · 12 ms · 2 sessions");
    expect(hostLine({ ...DEVBOX, latency_ms: null })).toBe("Rex · 2 sessions");
    expect(hostLine(NAS)).toBe("Rex · Not connected");
  });

  it("says why its sessions cannot be opened, in whole sentences", () => {
    expect(reason(DEVBOX)).toBeNull();
    for (const code of codes) {
      const why = reason({ ...NAS, state: rexFailed(code, "the peer went away") }) ?? "";
      expect(why).toMatch(/[.!?…]$/);
      if (code === "confirmation_required") expect(why).toBe(UNENCRYPTED);
      else expect(why.toLowerCase()).toContain("the peer went away");
    }
    expect(reason({ ...NAS, state: rexFailed("ssh", "Permission denied.") })).toMatch(
      /^Permission denied\. Try again to be asked/,
    );
    const changed = rexFailed("ssh", "Host key verification failed");
    expect(reason({ ...NAS, state: changed })).toMatch(/its key has changed/);
    expect(reason({ ...NAS, state: { kind: "waiting", error: "reset" } })).toContain("Reset.");
    const odd = rexHost("?odd", "odd", { state: rexFailed("bad_request", "ftp is not known") });
    expect(reason(odd)).toBe("The address cannot be used. Ftp is not known.");
  });

  it("says what can be done about it", () => {
    expect(next(DEVBOX)).toBeNull();
    expect(next(NAS)).toEqual({ kind: "connect", label: "Connect" });
    expect(next({ ...NAS, state: { kind: "absent" } })?.label).toBe("Retry");
    expect(next({ ...NAS, state: rexFailed("untrusted", "x") })).toEqual({
      kind: "connect",
      label: "Retry",
    });
    // A connection without encryption is never made by only trying again.
    expect(next({ ...NAS, state: rexFailed("confirmation_required", "x") })?.kind).toBe("agree");
    expect(next({ ...NAS, state: { kind: "connecting" } })).toBeNull();
    expect(next({ ...NAS, state: { kind: "waiting", error: "x" } })).toBeNull();
    // An address nothing can be reached at has nothing to try.
    const odd = rexHost("?odd", "odd", { state: rexFailed("bad_request", "x") });
    expect(usable(odd)).toBe(false);
    expect(next(odd)).toBeNull();
    expect(canDisconnect(DEVBOX)).toBe(true);
    expect(canDisconnect({ state: { kind: "connecting" } })).toBe(true);
    expect(canDisconnect(NAS)).toBe(false);
  });

  it("says with a dot whether it can be used", () => {
    expect(linkOf(DEVBOX)).toBe("up");
    expect(linkOf({ state: { kind: "connecting" } })).toBe("busy");
    expect(linkOf({ state: { kind: "waiting", error: "x" } })).toBe("busy");
    expect(linkOf(NAS)).toBe("down");
    expect(linkOf({ state: { kind: "absent" } })).toBe("down");
  });

  it("names the system it runs on", () => {
    expect(systemName("windows_amd64")).toBe("Windows");
    expect(systemName("linux_arm64")).toBe("Linux");
    expect(systemName("darwin_arm64")).toBe("macOS");
    expect(systemName("plan9_386")).toBe("plan9");
    expect(systemName("")).toBe("—");
  });
});

describe("which session of a server is on show", () => {
  const shown = { ...DEVBOX, on_show: "session:2" };

  it("is told by whose layout the windows show, before the server's own word", () => {
    expect(sessionOnShow(shown, shown.id)).toBe("session:2");
    // The windows show a workspace of their own already.
    expect(sessionOnShow(shown, null)).toBeNull();
    expect(sessionOnShow(shown, NAS.id)).toBeNull();
    expect(hostOnShow(rexList(NAS, shown), shown.id)).toBe(shown);
    expect(hostOnShow(rexList(NAS, shown), null)).toBeUndefined();
    expect(hostOnShow(null, shown.id)).toBeUndefined();
  });

  it("is the one marked first", () => {
    expect(firstMarked(shown, shown.id)).toBe(1);
    expect(firstMarked(shown, null)).toBe(0);
    expect(firstMarked(NAS, NAS.id)).toBe(0);
  });

  it("has a name when the server gave it none", () => {
    expect(sessionName({ label: "acme" })).toBe("acme");
    expect(sessionName({ label: "  " })).toBe("unnamed");
  });
});
