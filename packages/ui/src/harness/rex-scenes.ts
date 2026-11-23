/**
 * The scenes of the Rex servers: the Connect palette with servers in it, as they stand in
 * every way they can; the command palette's hosts; the selector in the top bar; and a
 * window whose layout a server keeps.
 */

import type { RexHost, RexHosts } from "../core/types.ts";
import type { ShellApp } from "../shell/boot.ts";
import type { FakeCoreOptions } from "./fake-core.ts";
import { FIXTURE_BINDINGS, FIXTURE_COMMANDS, HOST_COMMANDS } from "./fixture-commands.ts";
import type { Scene } from "./scene.ts";
import { CHROME_START_TIMEOUT_MS } from "./tuning.ts";

/** How often a scene looks again for what it waits for. */
const POLL_MS = 20;

const DEVBOX: RexHost = {
  id: "rex:ssh://devbox-01.example",
  label: "devbox-01",
  address: "ssh://mira@devbox-01.example",
  local: false,
  state: {
    kind: "connected",
    version: "0.1.0-preview.22",
    platform: "linux_amd64",
    api_revision: 19,
  },
  sessions: [
    { id: "session:1", label: "acme" },
    { id: "session:2", label: "ml-train" },
    { id: "session:3", label: "scratch" },
  ],
  on_show: null,
  latency_ms: 12,
};

const NAS: RexHost = {
  ...DEVBOX,
  id: "rex:https://home-nas.example",
  label: "home-nas",
  address: "https://home-nas.example",
  state: { kind: "idle" },
  sessions: [],
  latency_ms: null,
};

const CI: RexHost = {
  ...NAS,
  id: "rex:https://dev-ci.example",
  label: "dev-ci-runner",
  address: "https://dev-ci.example",
  state: { kind: "failed", code: "unreachable", error: "the host did not answer in time" },
};

const LAB: RexHost = {
  ...NAS,
  id: "rex:http://192.0.2.7:7297",
  label: "lab",
  address: "http://192.0.2.7:7297",
  state: {
    kind: "failed",
    code: "confirmation_required",
    error: "the connection would not be encrypted",
  },
};

const HOSTS: Omit<RexHosts, "revision"> = { local: true, hosts: [DEVBOX, NAS, CI, LAB] };

const WINDOW = {
  tabs: [2],
  commands: [...FIXTURE_COMMANDS, ...HOST_COMMANDS],
  bindings: FIXTURE_BINDINGS,
  rexHosts: HOSTS,
  workspaces: {
    active: "fixture",
    workspaces: [
      { name: "fixture", root: null },
      { name: "lume", root: "D:\\Code\\lume" },
      // A workspace that is called what a session of a server is.
      { name: "acme", root: null },
    ],
  },
} as const satisfies FakeCoreOptions;

/** A window that shows the session "acme" of the first server. */
const SERVED = {
  ...WINDOW,
  served: { backend: DEVBOX.id, name: "acme" },
  rexHosts: { ...HOSTS, hosts: [{ ...DEVBOX, on_show: "session:1" }, NAS, CI, LAB] },
  workspaces: { ...WINDOW.workspaces, active: "acme" },
} as const satisfies FakeCoreOptions;

async function until<T>(what: string, find: () => T | null | undefined): Promise<T> {
  const deadline = performance.now() + CHROME_START_TIMEOUT_MS;
  for (;;) {
    const found = find();
    if (found !== null && found !== undefined) return found;
    if (performance.now() > deadline) throw new Error(`${what} never happened`);
    await new Promise((resolve) => window.setTimeout(resolve, POLL_MS));
  }
}

/** Types into a field the way a user would, so that the component hears of it. */
function typeInto(field: HTMLInputElement, text: string): void {
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  set?.call(field, text);
  field.dispatchEvent(new Event("input", { bubbles: true }));
}

function press(field: HTMLElement, key: string, init: KeyboardEventInit = {}): void {
  field.dispatchEvent(
    new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }),
  );
}

/** Opens the Connect palette on one kind, with the server called `label` marked. */
async function connectOn(app: ShellApp, label: string): Promise<HTMLInputElement> {
  // Pins are kept in the page's storage: a scene starts from none.
  window.localStorage.removeItem("lume.connect.pins");
  await until("the servers arriving", () => app.rex.getState().list);
  app.connect.getState().open("rex");
  const field = await until("the Connect palette opening", () =>
    document.querySelector<HTMLInputElement>(".connect-input"),
  );
  // From the last row up to the server: wherever it stands, the arrows reach it.
  press(field, "Home");
  await until(`${label} being marked`, () => {
    const marked = document.querySelector('.connect-row[aria-selected="true"] .connect-row-name');
    if (marked?.textContent === label) return true;
    press(field, "ArrowDown");
    return null;
  });
  return field;
}

/** Opens the selector and waits for what it lists. */
async function openSelector(app: ShellApp): Promise<HTMLInputElement> {
  await until("the servers arriving", () => app.rex.getState().list);
  const button = await until("the selector's button", () =>
    document.querySelector<HTMLButtonElement>(".switcher-button"),
  );
  button.click();
  await until("its hosts being listed", () =>
    document.querySelectorAll(".selector-host").length === HOSTS.hosts.length + 1 ? true : null,
  );
  await until("the workspaces being listed", () =>
    document.querySelector(".selector-host .selector-line")?.textContent?.includes("3 workspaces")
      ? true
      : null,
  );
  return until("its field", () => document.querySelector<HTMLInputElement>(".selector-input"));
}

/** Looks at one host of the selector with the pointer, as hovering does. */
async function lookAt(host: string): Promise<void> {
  const row = await until("the host's row", () =>
    document.querySelector<HTMLElement>(`.selector-host[data-host="${host}"]`),
  );
  row.dispatchEvent(new PointerEvent("pointermove", { bubbles: true }));
  await until("the host being looked at", () =>
    row.getAttribute("aria-selected") === "true" ? true : null,
  );
}

function texts(page: ParentNode, selector: string): string[] {
  return [...page.querySelectorAll(selector)].map((found) => found.textContent ?? "");
}

function wanted(what: string, is: unknown, should: unknown): string[] {
  return JSON.stringify(is) === JSON.stringify(should)
    ? []
    : [`${what}: ${JSON.stringify(is)}, not ${JSON.stringify(should)}`];
}

/** The scenes of the Rex servers, in the order they are run. */
export const REX_SCENES: readonly Scene[] = [
  {
    name: "connect-rex",
    window: WINDOW,
    prepare: async (app) => {
      await connectOn(app, "home-nas");
    },
    check: (page) => [
      ...wanted("the servers", texts(page, ".connect-row-line"), [
        "3 sessions",
        "Not connected",
        "Unreachable",
        "Not encrypted",
      ]),
      ...wanted("what can be done", texts(page, ".connect-act-label"), ["Connect", "Copy address"]),
    ],
  },
  {
    // The marked server's sessions with the arrow keys among them, the second marked.
    name: "connect-rex-sessions",
    window: WINDOW,
    prepare: async (app) => {
      const field = await connectOn(app, "devbox-01");
      press(field, "Tab");
      await until("the sessions being entered", () =>
        document.querySelector(".connect-sessions[data-inside]"),
      );
      press(field, "ArrowDown");
      await until("the second being marked", () =>
        document.querySelector('.connect-session[aria-selected="true"]')?.textContent === "ml-train"
          ? true
          : null,
      );
    },
    check: (page) => [
      ...wanted("the numbers", texts(page, ".connect-stats dd"), ["12 ms", "Linux", "19"]),
      ...wanted(
        "what Enter does",
        page.querySelector(".connect-primary .connect-act-label")?.textContent,
        "Attach to ml-train",
      ),
      ...wanted(
        "what the field points at",
        page.querySelector(".connect-input")?.getAttribute("aria-activedescendant"),
        "connect-session-1",
      ),
    ],
  },
  {
    name: "connect-rex-refused",
    window: WINDOW,
    prepare: async (app) => {
      await connectOn(app, "dev-ci-runner");
    },
    check: (page) => [
      ...wanted("why", texts(page, ".connect-call"), [
        "Unreachable. The host did not answer in time.",
      ]),
      ...wanted("what can be done", texts(page, ".connect-act-label"), ["Retry", "Copy address"]),
    ],
  },
  {
    // Enter alone does not agree to a connection without encryption.
    name: "connect-rex-unencrypted",
    window: WINDOW,
    prepare: async (app, core) => {
      const field = await connectOn(app, "lab");
      press(field, "Enter");
      await new Promise((resolve) => window.setTimeout(resolve, POLL_MS * 5));
      if (core.ran.length > 0) throw new Error("Enter agreed to a connection without encryption");
      press(field, "Enter", { ctrlKey: true });
      await until("the agreement being sent", () => (core.ran.length > 0 ? true : null));
      const ran = JSON.stringify(core.ran);
      if (!ran.includes('"confirmed":true')) throw new Error(`what was run: ${ran}`);
    },
    check: (page) =>
      wanted("what can be done", texts(page, ".connect-act-label"), [
        "Connect without encryption",
        "Copy address",
      ]),
  },
  {
    name: "palette-hosts",
    window: WINDOW,
    prepare: async (app) => {
      await until("the servers arriving", () => app.rex.getState().list);
      await until("the profiles arriving", () => app.profiles.getState().list);
      app.palette.getState().open();
      const field = await until("the palette's field", () =>
        document.querySelector<HTMLInputElement>(".palette-input"),
      );
      typeInto(field, "#");
      await until("the hosts being listed", () =>
        document.querySelector('.palette-row .palette-host-icon[data-kind="rex"]') ? true : null,
      );
    },
    check: (page) =>
      wanted("the servers and sessions", texts(page, ".palette-row-text").slice(0, 5), [
        "devbox-01",
        "acme",
        "ml-train",
        "scratch",
        "home-nas",
      ]),
  },
  {
    // The selector as it opens on a workspace of the application's own.
    name: "selector",
    window: WINDOW,
    prepare: async (app) => {
      await openSelector(app);
    },
    check: (page) => [
      ...wanted(
        "the button",
        page.querySelector(".switcher-button")?.getAttribute("aria-label"),
        "This PC, fixture",
      ),
      ...wanted("the workspaces", texts(page, ".selector-row .selector-name"), [
        "fixture",
        "lume",
        "acme",
      ]),
      ...wanted("what is on show", texts(page, ".selector-row[data-on] .selector-name"), [
        "fixture",
      ]),
      ...(page.querySelector(".selector-input") === document.activeElement
        ? []
        : ["the selector's field does not have the keyboard"]),
    ],
  },
  {
    // A server's session on show, another of its sessions being renamed.
    name: "selector-rex",
    window: SERVED,
    prepare: async (app) => {
      const field = await openSelector(app);
      await until("the sessions being listed", () =>
        document.querySelectorAll(".selector-row").length === 3 ? true : null,
      );
      press(field, "ArrowDown");
      await until("the second being marked", () =>
        document.querySelector('.selector-row[data-marked][data-row="session:session:2"]'),
      );
      press(field, "F2");
      await until("the name's field", () => document.querySelector(".selector-rename"));
    },
    check: (page) => [
      ...wanted(
        "the button",
        page.querySelector(".switcher-button")?.getAttribute("aria-label"),
        "devbox-01, acme",
      ),
      ...wanted("how near", page.querySelector(".switcher-meta")?.textContent, "12 ms"),
      ...wanted("what is on show", texts(page, ".selector-row[data-on] .selector-right"), [
        "Attached",
      ]),
      ...wanted("what can be done", texts(page, ".selector-act-label"), [
        "New session on devbox-01",
        "Disconnect",
      ]),
      ...(page.querySelector(".selector-rename") === document.activeElement
        ? []
        : ["the field of the name does not have the keyboard"]),
    ],
  },
  {
    // This machine's workspaces while a session called as one of them is is on show.
    name: "selector-served",
    window: SERVED,
    prepare: async (app) => {
      await openSelector(app);
      await lookAt("pc");
    },
    check: (page) => [
      ...wanted("what is on show", texts(page, ".selector-row[data-on]"), []),
      ...wanted("what can be done", texts(page, ".selector-act-label"), [
        "Open folder…",
        "New workspace…",
      ]),
    ],
  },
  {
    name: "selector-offline",
    window: WINDOW,
    prepare: async (app) => {
      const field = await openSelector(app);
      await lookAt(CI.id);
      press(field, "Tab");
      await until("the hosts' column having the arrows", () =>
        document.querySelector(".selector-host[data-marked]"),
      );
    },
    check: (page) => [
      ...wanted("why", texts(page, ".selector-why"), [
        "Unreachable. The host did not answer in time.",
      ]),
      ...wanted("what can be done", texts(page, ".selector-act-label"), ["Retry"]),
    ],
  },
  {
    // What was typed is not there, and can be made on the host looked at.
    name: "selector-create",
    window: SERVED,
    prepare: async (app) => {
      const field = await openSelector(app);
      typeInto(field, "nightly");
      await until("making it being offered", () =>
        document.querySelector('.selector-act[data-act="create"]'),
      );
    },
    check: (page) =>
      wanted("what can be done", texts(page, ".selector-act-label").slice(0, 1), [
        "Create “nightly” on devbox-01",
      ]),
  },
  {
    // A layout a server keeps, with something in it that cannot be shown, while the
    // connection is being made again.
    name: "served-workspace",
    window: { ...SERVED, placeholder: { pane: "p2", label: "Notes", kind: "com.example.notes" } },
    prepare: async (app, core) => {
      await until("the servers arriving", () => app.rex.getState().list);
      const lost = { kind: "waiting", error: "the connection was closed" } as const;
      core.rexHostsChanged([{ ...DEVBOX, on_show: "session:1", state: lost }, NAS, CI, LAB]);
      await until("the connection being said to be lost", () =>
        document.querySelector('.switcher-dot[data-link="busy"]'),
      );
    },
    check: (page) => [
      ...wanted("how near", texts(page, ".switcher-meta"), []),
      ...wanted("the turning rings", texts(page, '.switcher-dot[data-link="busy"]').length, 2),
      ...wanted("the status bar", texts(page, ".status-served-text"), [
        "devbox-01 · session acme · 2 panes",
      ]),
      ...wanted("what cannot be shown", texts(page, ".panel-placeholder").length, 1),
    ],
  },
];
