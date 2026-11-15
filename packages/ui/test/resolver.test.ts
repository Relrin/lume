import { describe, expect, it } from "vitest";

import type { KeyBinding } from "../src/core/types.ts";
import { KeyResolver, type KeyOutcome } from "../src/keys/resolver.ts";
import { KEY_SEQUENCE_TIMEOUT_MS } from "../src/tuning.ts";

function binding(keys: string, command: string, when = "", args: unknown = null): KeyBinding {
  return {
    keys: keys.split(" "),
    command,
    args,
    when: when === "" ? [] : when.split(" && "),
    source: { layer: "default" },
  };
}

function resolver(...bindings: KeyBinding[]): KeyResolver {
  const made = new KeyResolver();
  made.setBindings(bindings);
  return made;
}

function contexts(...names: string[]): ReadonlySet<string> {
  return new Set(names);
}

const TERMINAL = contexts("terminal");
const PASS: KeyOutcome = { kind: "pass" };
const PENDING: KeyOutcome = { kind: "pending" };

function run(command: string, args: unknown = null): KeyOutcome {
  return { kind: "run", command, args };
}

describe("a chord that is bound", () => {
  it("runs its command with the binding's arguments", () => {
    const keys = resolver(
      binding("ctrl+shift+t", "lume.tab.new"),
      binding("ctrl+alt+3", "lume.tab.activate", "", { index: 3 }),
    );
    expect(keys.resolve("ctrl+shift+t", TERMINAL, 0)).toEqual(run("lume.tab.new"));
    expect(keys.resolve("ctrl+alt+3", TERMINAL, 0)).toEqual(run("lume.tab.activate", { index: 3 }));
  });

  it("passes a chord nothing is bound to", () => {
    const keys = resolver(binding("ctrl+shift+t", "lume.tab.new"));
    expect(keys.resolve("ctrl+t", TERMINAL, 0)).toEqual(PASS);
    expect(keys.resolve("t", TERMINAL, 0)).toEqual(PASS);
    expect(new KeyResolver().resolve("ctrl+shift+t", TERMINAL, 0)).toEqual(PASS);
  });
});

describe("where a binding holds", () => {
  const keys = resolver(
    binding("ctrl+shift+c", "lume.terminal.copy", "terminal"),
    binding("shift+pageup", "lume.terminal.scroll-page-up", "!alt-screen && terminal"),
    binding("escape", "lume.palette.close", "palette"),
    binding("ctrl+w", "lume.panel.close", "panel:lume.tabs"),
  );

  it("holds where every context it names holds", () => {
    expect(keys.resolve("ctrl+shift+c", TERMINAL, 0)).toEqual(run("lume.terminal.copy"));
    expect(keys.resolve("ctrl+shift+c", contexts("terminal", "floating"), 0).kind).toBe("run");
    expect(keys.resolve("ctrl+shift+c", contexts("palette"), 0)).toEqual(PASS);
    expect(keys.resolve("ctrl+shift+c", contexts(), 0)).toEqual(PASS);
    expect(keys.resolve("escape", TERMINAL, 0)).toEqual(PASS);
    expect(keys.resolve("escape", contexts("palette"), 0)).toEqual(run("lume.palette.close"));
    expect(keys.resolve("ctrl+w", contexts("panel:lume.tabs"), 0).kind).toBe("run");
    expect(keys.resolve("ctrl+w", contexts("panel:other"), 0)).toEqual(PASS);
  });

  it("does not hold where a context it negates holds", () => {
    expect(keys.resolve("shift+pageup", TERMINAL, 0).kind).toBe("run");
    expect(keys.resolve("shift+pageup", contexts("terminal", "alt-screen"), 0)).toEqual(PASS);
    expect(keys.resolve("shift+pageup", contexts("alt-screen"), 0)).toEqual(PASS);
  });
});

describe("which of several bindings wins", () => {
  it("is the one that names more contexts", () => {
    const keys = resolver(
      binding("ctrl+shift+f", "lume.search.open", "terminal"),
      binding("ctrl+shift+f", "lume.search.next", "search && terminal"),
      binding("ctrl+shift+f", "lume.everywhere"),
    );
    expect(keys.resolve("ctrl+shift+f", contexts("terminal", "search"), 0)).toEqual(
      run("lume.search.next"),
    );
    expect(keys.resolve("ctrl+shift+f", TERMINAL, 0)).toEqual(run("lume.search.open"));
    expect(keys.resolve("ctrl+shift+f", contexts("settings"), 0)).toEqual(run("lume.everywhere"));
  });

  it("is the later of two that name as many", () => {
    const keys = resolver(
      binding("ctrl+shift+t", "lume.tab.new"),
      binding("ctrl+shift+t", "user.own"),
      binding("ctrl+shift+k", "lume.terminal.clear", "terminal"),
      binding("ctrl+shift+k", "user.clear", "floating"),
    );
    expect(keys.resolve("ctrl+shift+t", TERMINAL, 0)).toEqual(run("user.own"));
    expect(keys.resolve("ctrl+shift+k", contexts("terminal", "floating"), 0)).toEqual(
      run("user.clear"),
    );
    expect(keys.resolve("ctrl+shift+k", TERMINAL, 0)).toEqual(run("lume.terminal.clear"));
  });

  it("needs nothing for a default the user took away: it is not among the bindings", () => {
    // `"ctrl+shift+w" = "unbound"` reaches the page as the absence of the binding.
    const keys = resolver(binding("ctrl+shift+t", "lume.tab.new"));
    expect(keys.resolve("ctrl+shift+w", TERMINAL, 0)).toEqual(PASS);
  });
});

describe("a sequence of chords", () => {
  function tmuxLike(): KeyResolver {
    return resolver(
      binding("ctrl+a c", "lume.tab.new"),
      binding("ctrl+a shift+5", "lume.pane.split-right"),
      binding("ctrl+a ctrl+a x", "lume.pane.close"),
      binding("c", "lume.alone", "palette"),
    );
  }

  it("waits after its first chord and runs on its last", () => {
    const keys = tmuxLike();
    expect(keys.resolve("ctrl+a", TERMINAL, 0)).toEqual(PENDING);
    expect(keys.pending(10)).toBe(true);
    expect(keys.resolve("c", TERMINAL, 300)).toEqual(run("lume.tab.new"));
    expect(keys.pending(300)).toBe(false);
    // Three chords.
    expect(keys.resolve("ctrl+a", TERMINAL, 1000)).toEqual(PENDING);
    expect(keys.resolve("ctrl+a", TERMINAL, 1200)).toEqual(PENDING);
    expect(keys.resolve("x", TERMINAL, 1400)).toEqual(run("lume.pane.close"));
  });

  it("goes before a shorter binding of its last chord", () => {
    const keys = tmuxLike();
    const both = contexts("terminal", "palette");
    expect(keys.resolve("c", both, 0)).toEqual(run("lume.alone"));
    expect(keys.resolve("ctrl+a", both, 10)).toEqual(PENDING);
    expect(keys.resolve("c", both, 20)).toEqual(run("lume.tab.new"));
  });

  it("is forgotten when the next chord takes too long", () => {
    const keys = tmuxLike();
    expect(keys.resolve("ctrl+a", TERMINAL, 0)).toEqual(PENDING);
    expect(keys.pending(KEY_SEQUENCE_TIMEOUT_MS)).toBe(true);
    expect(keys.pending(KEY_SEQUENCE_TIMEOUT_MS + 1)).toBe(false);
    expect(keys.resolve("c", TERMINAL, KEY_SEQUENCE_TIMEOUT_MS + 1)).toEqual(PASS);
    // Just in time is in time, and each chord has the whole time again.
    expect(keys.resolve("ctrl+a", TERMINAL, 5000)).toEqual(PENDING);
    expect(keys.resolve("ctrl+a", TERMINAL, 5000 + KEY_SEQUENCE_TIMEOUT_MS)).toEqual(PENDING);
    expect(keys.resolve("x", TERMINAL, 5000 + 2 * KEY_SEQUENCE_TIMEOUT_MS)).toEqual(
      run("lume.pane.close"),
    );
  });

  it("passes a chord that completes nothing and begins afresh", () => {
    const keys = tmuxLike();
    expect(keys.resolve("ctrl+a", TERMINAL, 0)).toEqual(PENDING);
    expect(keys.resolve("q", TERMINAL, 10)).toEqual(PASS);
    expect(keys.pending(10)).toBe(false);
    expect(keys.resolve("c", TERMINAL, 20)).toEqual(PASS);
    expect(keys.resolve("ctrl+a", TERMINAL, 30)).toEqual(PENDING);
    expect(keys.resolve("shift+5", TERMINAL, 40)).toEqual(run("lume.pane.split-right"));
  });

  it("is not begun where it does not hold", () => {
    const keys = resolver(binding("ctrl+k ctrl+s", "lume.settings.open", "terminal"));
    expect(keys.resolve("ctrl+k", contexts("palette"), 0)).toEqual(PASS);
    expect(keys.resolve("ctrl+k", TERMINAL, 0)).toEqual(PENDING);
    // The contexts are those of each chord as it is pressed.
    expect(keys.resolve("ctrl+s", contexts("palette"), 10)).toEqual(PASS);
  });

  it("gives way to a binding its first chord completes", () => {
    const keys = resolver(
      binding("ctrl+k", "lume.terminal.clear"),
      binding("ctrl+k ctrl+s", "lume.settings.open"),
    );
    expect(keys.resolve("ctrl+k", TERMINAL, 0)).toEqual(run("lume.terminal.clear"));
    expect(keys.pending(0)).toBe(false);
    expect(keys.resolve("ctrl+s", TERMINAL, 10)).toEqual(PASS);
  });

  it("is forgotten when asked, and when the bindings change", () => {
    const keys = tmuxLike();
    keys.resolve("ctrl+a", TERMINAL, 0);
    keys.reset();
    expect(keys.resolve("c", TERMINAL, 10)).toEqual(PASS);
    keys.resolve("ctrl+a", TERMINAL, 20);
    keys.setBindings([binding("ctrl+a c", "lume.tab.new")]);
    expect(keys.pending(30)).toBe(false);
    expect(keys.resolve("c", TERMINAL, 30)).toEqual(PASS);
  });
});
