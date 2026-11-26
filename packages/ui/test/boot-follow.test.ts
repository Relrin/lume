import { DEFAULT_TOKENS } from "@lume/tokens";
import { describe, expect, it } from "vitest";

import type {
  CommandSpec,
  ConfigView,
  LayoutView,
  Notice,
  ProfileList,
  RexHosts,
} from "../src/core/types.ts";
import type { PaneViews } from "../src/panes/views.ts";
import { askSlowly } from "../src/shell/boot/core.ts";
import { followCore, type ConfirmNotice, type FollowDeps } from "../src/shell/boot/follow.ts";
import { late } from "../src/shell/boot/late.ts";
import { createStores } from "../src/shell/boot/stores.ts";
import { settings } from "./config-support.ts";
import { oneTab, pane, viewOf } from "./layout-support.ts";

function configView(revision: number): ConfigView {
  return {
    revision,
    platform: "windows",
    scheme: "dark",
    theme: { id: "lume-dark", scheme: "dark" },
    config: settings({
      appearance: { "pane-header": "always" },
      terminal: { "scrollback-lines": 10_000, "cursor-style": "block", "cursor-blink": true },
      layout: { "tab-placement": "top" },
      renderer: { tier: "auto" },
    }),
    sources: {},
    problems: [],
    tokens: DEFAULT_TOKENS,
    keymap: { bindings: [], removed: [] },
    layouts: [],
    schema_revision: 1,
    title_bar: "system",
  };
}

/** A promise and the means to settle it, for an answer the test gives when it likes. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

/** Lets the answers already given be taken in. */
const settle = () => new Promise((done) => setTimeout(done));

/** The stores a page has, a stand-in for the core, and a record of what was done. */
function page() {
  const stores = createStores();
  const done: string[] = [];
  const pushed: { level: string; message: string; action?: unknown }[] = [];
  const asked = {
    layout: [] as ReturnType<typeof deferred<LayoutView>>[],
    commands: [] as ReturnType<typeof deferred<CommandSpec[]>>[],
  };
  const confirmed: ConfirmNotice[] = [];
  const deps: FollowDeps = {
    ...stores,
    client: {
      layoutView: () => {
        const answer = deferred<LayoutView>();
        asked.layout.push(answer);
        return answer.promise;
      },
      commandsList: () => {
        const answer = deferred<CommandSpec[]>();
        asked.commands.push(answer);
        return answer.promise;
      },
    },
    notices: {
      push: (level, message, action) =>
        void pushed.push(action === undefined ? { level, message } : { level, message, action }),
    },
    prompts: { withdraw: (ask) => void done.push(`withdraw ${ask}`) },
    contributions: { announced: (revision) => void done.push(`announced ${revision}`) },
    configChanged: (view) => void done.push(`config ${view.revision}`),
    restyle: () => void done.push("restyle"),
    closeRequested: () => void done.push("close"),
    confirmRequested: (notice) => void confirmed.push(notice),
  };
  return { stores, follow: followCore(deps), done, pushed, asked, confirmed };
}

const LAYOUT = viewOf(oneTab(pane("p1"), "p1"));

describe("following the core", () => {
  it("takes a new layout in whole", () => {
    const { stores, follow } = page();
    follow({ type: "layout_reset", ...LAYOUT });
    const state = stores.layout.getState();
    expect(state.ready).toBe(true);
    expect(state.epoch).toBe(LAYOUT.epoch);
  });

  it("asks for the whole layout once when changes are missing, however many say so", async () => {
    const { stores, follow, asked } = page();
    follow({ type: "layout_reset", ...LAYOUT });
    const gap: Notice = {
      type: "layout_changed",
      epoch: LAYOUT.epoch,
      revision: LAYOUT.revision + 5,
      changes: [],
    };
    follow(gap);
    follow({ ...gap, revision: LAYOUT.revision + 6 });
    expect(asked.layout).toHaveLength(1);

    const again = viewOf(oneTab(pane("p2"), "p2"), "w1", LAYOUT.epoch + 1, 0);
    asked.layout[0]!.resolve(again);
    await settle();
    expect(stores.layout.getState().epoch).toBe(again.epoch);
    // Once that answer is in, a gap asks again.
    follow({ ...gap, epoch: again.epoch, revision: 9 });
    expect(asked.layout).toHaveLength(2);
  });

  it("says so when the whole layout cannot be read again", async () => {
    const { follow, asked, pushed } = page();
    follow({ type: "layout_reset", ...LAYOUT });
    follow({ type: "layout_changed", epoch: LAYOUT.epoch, revision: 7, changes: [] });
    asked.layout[0]!.reject(new Error("gone"));
    await settle();
    expect(pushed).toEqual([
      { level: "error", message: "The layout could not be read again: gone" },
    ]);
  });

  it("takes a newer configuration in, and passes over one it holds", () => {
    const { stores, follow, done } = page();
    follow({ type: "config_changed", revision: 2, changed: [], view: configView(2) });
    expect(stores.config.getState().view?.revision).toBe(2);
    follow({ type: "config_changed", revision: 2, changed: [], view: configView(2) });
    follow({ type: "config_changed", revision: 1, changed: [], view: configView(1) });
    expect(done).toEqual(["config 2"]);
  });

  it("names what is wrong with a configuration it was not given", () => {
    const { follow, pushed } = page();
    follow({
      type: "config_error",
      problems: [
        {
          kind: "invalid_value",
          path: "font.size",
          file: "C:\\Users\\me\\lume\\config.toml",
          line: 3,
          column: 1,
          message: "font.size must be a number",
          suggestion: null,
        },
      ],
    });
    expect(pushed).toHaveLength(1);
    expect(pushed[0]!.level).toBe("error");
    expect(pushed[0]!.message).toContain("the one before it is still in use");
    expect(pushed[0]!.message).toContain("config.toml:3: font.size must be a number");
  });

  it("keeps what is wrong with the files until a configuration is taken again", () => {
    const { stores, follow } = page();
    const problem = {
      kind: "syntax" as const,
      path: "",
      file: "config.toml",
      line: 1,
      column: 1,
      message: "expected `]`",
      suggestion: null,
    };
    follow({ type: "config_error", problems: [problem] });
    expect(stores.config.getState().failed).toEqual([problem]);
    follow({ type: "config_changed", revision: 2, changed: [], view: configView(2) });
    expect(stores.config.getState().failed).toEqual([]);
  });

  it("offers to apply a layout file that changed, saying what happens to running panes", () => {
    const { follow, pushed } = page();
    follow({
      type: "layout_file_changed",
      tab: "t4",
      layout: "dev",
      name: "Lume dev",
      leaving: ["p7", "p8"],
      busy: ["p8"],
    });
    expect(pushed).toHaveLength(1);
    expect(pushed[0]!.level).toBe("warning");
    expect(pushed[0]!.message).toBe(
      "The layout “Lume dev” changed on disk. Applying it: 1 idle pane will close; 1 pane still running will be put away, not ended.",
    );
    expect(pushed[0]!.action).toEqual({
      label: "Apply",
      command: "lume.layout.reapply",
      args: { tab: "t4" },
    });

    follow({
      type: "layout_file_changed",
      tab: "t4",
      layout: "dev",
      name: "Lume dev",
      leaving: [],
      busy: [],
    });
    expect(pushed[1]!.level).toBe("info");
    expect(pushed[1]!.message).toBe("The layout “Lume dev” changed on disk.");
  });

  it("styles the terminals again for newer profiles only", () => {
    const { stores, follow, done } = page();
    const profiles = { profiles: [], default: null, notes: [], looked: true };
    follow({ type: "profiles_changed", revision: 2, ...profiles } as Notice);
    follow({ type: "profiles_changed", revision: 1, ...profiles } as Notice);
    expect(stores.profiles.getState().list?.revision).toBe(2);
    expect(done).toEqual(["restyle"]);
  });

  it("keeps the window's state, the servers and the host's link", () => {
    const { stores, follow } = page();
    follow({ type: "window_state", maximised: true, fullscreen: false, focused: true });
    follow({ type: "rex_hosts_changed", revision: 1, hosts: [], local: true });
    follow({ type: "host_link", connected: false });
    expect(stores.windowState.getState()).toMatchObject({ maximised: true, focused: true });
    expect(stores.rex.getState().list).toMatchObject({ revision: 1, local: true });
    expect(stores.hostLink.getState().connected).toBe(false);
  });

  it("shows the core's notices and withdraws what is no longer asked", () => {
    const { follow, pushed, done } = page();
    follow({ type: "notice", level: "warning", message: "careful" });
    follow({ type: "sign_in_withdrawn", ask: 4 });
    expect(pushed).toEqual([{ level: "warning", message: "careful" }]);
    expect(done).toEqual(["withdraw 4"]);
  });

  it("hands on what the window is asked to close or confirm", () => {
    const { follow, done, confirmed } = page();
    const confirm: ConfirmNotice = {
      type: "confirm_requested",
      title: "Close?",
      message: "A program runs.",
      confirm: "Close",
      command: "lume.pane.close",
      args: { pane: "p1" },
    };
    follow({ type: "close_requested" });
    follow(confirm);
    expect(done).toEqual(["close"]);
    expect(confirmed).toEqual([confirm]);
  });

  it("tells of a newer version of what the plugins add", () => {
    const { follow, done } = page();
    follow({ type: "contributions_changed", revision: 3 });
    expect(done).toEqual(["announced 3"]);
  });

  it("reads the commands again when they change", async () => {
    const { stores, follow, asked, pushed } = page();
    follow({ type: "commands_changed" });
    const command = { id: "lume.tab.new", title: "New tab" } as unknown as CommandSpec;
    asked.commands[0]!.resolve([command]);
    await settle();
    expect(stores.commandStore.getState().commands).toEqual([command]);

    follow({ type: "commands_changed" });
    asked.commands[1]!.reject(new Error("no core"));
    await settle();
    expect(pushed).toEqual([
      { level: "error", message: "The commands could not be read again: no core" },
    ]);
  });

  it("leaves alone what is said to the settings window", () => {
    const { stores, follow, done, pushed } = page();
    const before = stores.layout.getState();
    follow({ type: "settings_show", section: "fonts" });
    follow({ type: "plugins_changed" });
    expect(done).toEqual([]);
    expect(pushed).toEqual([]);
    expect(stores.layout.getState()).toBe(before);
  });
});

describe("what the page asks for as it starts", () => {
  /** A core that answers the lists when the test says. */
  function slowCore() {
    const profiles = deferred<ProfileList>();
    const rex = deferred<RexHosts>();
    return {
      profiles,
      rex,
      client: { profilesList: () => profiles.promise, rexHosts: () => rex.promise },
    };
  }
  const list = (revision: number): ProfileList => ({
    revision,
    profiles: [],
    default: null,
    notes: [],
    looked: true,
  });

  it("styles the views with profiles that came before they were made, once they are", async () => {
    const stores = createStores();
    const core = slowCore();
    const views = late<PaneViews>("the views");
    askSlowly(core.client, stores, views, { push: () => undefined });
    core.profiles.resolve(list(1));
    await settle();
    expect(stores.profiles.getState().list?.revision).toBe(1);
    let restyled = 0;
    views.set({ restyle: () => void restyled++ } as unknown as PaneViews);
    expect(restyled).toBe(1);
  });

  it("styles views that are there at once, and says when a list cannot be had", async () => {
    const stores = createStores();
    const core = slowCore();
    const views = late<PaneViews>("the views");
    let restyled = 0;
    views.set({ restyle: () => void restyled++ } as unknown as PaneViews);
    const pushed: string[] = [];
    askSlowly(core.client, stores, views, { push: (_level, message) => void pushed.push(message) });
    core.profiles.resolve(list(1));
    core.rex.reject(new Error("refused"));
    await settle();
    expect(restyled).toBe(1);
    expect(pushed).toEqual(["The Rex servers could not be read: refused"]);
  });
});
