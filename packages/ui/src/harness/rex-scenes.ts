/**
 * The scenes of the Rex servers: the Connect palette with servers in it, as they stand in
 * every way they can; the command palette's hosts; the selector in the top bar; and a
 * window whose layout a server keeps.
 */

import type { RexHost, RexHosts } from "../core/types.ts";
import type { ShellApp } from "../shell/boot/index.ts";
import type { FakeCoreOptions } from "./fake-core.ts";
import { FIXTURE_BINDINGS, FIXTURE_COMMANDS, HOST_COMMANDS } from "./fixture-commands.ts";
import type { Scene } from "./scene.ts";
import { CHROME_START_TIMEOUT_MS } from "./tuning.ts";

/** How often a scene looks again for what it waits for. */
const POLL_MS = 20;

export const DEVBOX: RexHost = {
  id: "rex:ssh://devbox-01.example",
  label: "devbox-01",
  address: "ssh://mira@devbox-01.example",
  local: false,
  written: true,
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

export const NAS: RexHost = {
  ...DEVBOX,
  id: "rex:https://home-nas.example",
  label: "home-nas",
  address: "https://home-nas.example",
  state: { kind: "idle" },
  sessions: [],
  latency_ms: null,
};

export const CI: RexHost = {
  ...NAS,
  id: "rex:https://dev-ci.example",
  label: "dev-ci-runner",
  address: "https://dev-ci.example",
  state: { kind: "failed", code: "unreachable", error: "the host did not answer in time" },
};

export const LAB: RexHost = {
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

export const WINDOW = {
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
export const SERVED = {
  ...WINDOW,
  served: { backend: DEVBOX.id, name: "acme" },
  rexHosts: { ...HOSTS, hosts: [{ ...DEVBOX, on_show: "session:1" }, NAS, CI, LAB] },
  workspaces: { ...WINDOW.workspaces, active: "acme" },
} as const satisfies FakeCoreOptions;

/** The same window, of a server with as many sessions as one may have. */
const CROWDED = {
  ...SERVED,
  rexHosts: {
    ...HOSTS,
    hosts: [
      {
        ...DEVBOX,
        sessions: Array.from({ length: 1024 }, (_, at) => ({
          id: `session:${at}`,
          label: `run ${at}`,
        })),
        on_show: "session:900",
      },
      NAS,
      CI,
      LAB,
    ],
  },
} as const satisfies FakeCoreOptions;

export async function until<T>(what: string, find: () => T | null | undefined): Promise<T> {
  const deadline = performance.now() + CHROME_START_TIMEOUT_MS;
  for (;;) {
    const found = find();
    if (found !== null && found !== undefined) return found;
    if (performance.now() > deadline) throw new Error(`${what} never happened`);
    await new Promise((resolve) => window.setTimeout(resolve, POLL_MS));
  }
}

/** Types into a field the way a user would, so that the component hears of it. */
export function typeInto(field: HTMLInputElement, text: string): void {
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

export function texts(page: ParentNode, selector: string): string[] {
  return [...page.querySelectorAll(selector)].map((found) => found.textContent ?? "");
}

export function wanted(what: string, is: unknown, should: unknown): string[] {
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
      ...wanted("what can be done", texts(page, ".connect-act-label"), [
        "Connect",
        "Copy address",
        "Edit…",
      ]),
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
      ...wanted("what can be done", texts(page, ".connect-act-label"), [
        "Retry",
        "Copy address",
        "Edit…",
      ]),
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
        "Edit…",
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
      ...wanted("what can be done", texts(page, ".selector-side .selector-act-label"), [
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
      ...wanted("what can be done", texts(page, ".selector-side .selector-act-label"), [
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
      ...wanted("what can be done", texts(page, ".selector-side .selector-act-label"), [
        "Retry",
        "Edit connection…",
      ]),
    ],
  },
  {
    // A server being called something else, where it is listed.
    name: "selector-rename-host",
    window: WINDOW,
    prepare: async (app) => {
      const field = await openSelector(app);
      await lookAt(NAS.id);
      press(field, "Tab");
      await until("the hosts' column having the arrows", () =>
        document.querySelector(".selector-host[data-marked]"),
      );
      press(field, "F2");
      const name = await until("the name's field", () =>
        document.querySelector<HTMLInputElement>(".selector-rename"),
      );
      typeInto(name, "Lab");
      await until("the name being refused", () =>
        document.querySelector('.selector-rename[aria-invalid="true"]'),
      );
    },
    check: (page) =>
      wanted(
        "why",
        page.querySelector(".selector-rename")?.getAttribute("title"),
        "That name is taken",
      ),
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
      wanted("what can be done", texts(page, ".selector-side .selector-act-label").slice(0, 1), [
        "Create “nightly” on devbox-01",
      ]),
  },
  {
    // More sessions than anyone reads, in the three places that list them. What is left
    // on show is what was typed for in the selector.
    name: "crowded-server",
    window: CROWDED,
    prepare: async (app) => {
      const said = (what: string, is: unknown, should: unknown) => {
        const problems = wanted(what, is, should);
        if (problems.length > 0) throw new Error(problems.join("; "));
      };
      const gone = (selector: string) =>
        until("its going", () => (document.querySelector(selector) ? null : true));

      const connect = await connectOn(app, "devbox-01");
      const listed = () => texts(document, ".connect-session-name");
      const marked = () =>
        document.querySelector('.connect-session[aria-selected="true"]')?.textContent;
      said("the sessions of the column", [listed().length, listed()[198]], [200, "run 198"]);
      said("what is marked", marked(), "run 900Attached");
      said("the rest", texts(document, ".connect-sessions .connect-note"), ["and 824 more"]);
      press(connect, "Tab");
      await until("the sessions being entered", () =>
        document.querySelector(".connect-sessions[data-inside]"),
      );
      // After the last that is listed comes the first.
      press(connect, "ArrowDown");
      await until("the first being marked", () => (marked() === "run 0" ? true : null));
      press(connect, "ArrowUp");
      await until("the last being marked", () => (marked() === "run 900Attached" ? true : null));
      app.connect.getState().close();
      await gone(".connect");

      app.palette.getState().open();
      const palette = await until("the palette's field", () =>
        document.querySelector<HTMLInputElement>(".palette-input"),
      );
      typeInto(palette, "#");
      await until("the hosts being listed", () =>
        document.querySelectorAll(".palette-row").length === 50 ? true : null,
      );
      const rows = texts(document, ".palette-row-text");
      said("the hosts of the palette", [rows[48], rows[49]], ["run 47", "run 900"]);
      const chip = Number(
        document.querySelector('.palette-chip[aria-pressed="true"] .palette-chip-count')
          ?.textContent,
      );
      said("the rest", texts(document, ".palette-more"), [`and ${chip - 50} more`]);
      if (chip < 1024) throw new Error(`the chip counts ${chip}`);
      app.palette.getState().close();
      await gone(".palette");

      const selector = await openSelector(app);
      await until("the sessions being listed", () =>
        document.querySelectorAll(".selector-row").length === 200 ? true : null,
      );
      said("the rest", texts(document, ".selector-rows .selector-note"), ["and 824 more"]);
      said(
        "what is marked",
        texts(document, '.selector-row[aria-selected="true"] .selector-name'),
        ["run 900"],
      );
      typeInto(selector, "run 102");
      await until("the list being narrowed", () =>
        document.querySelectorAll(".selector-row").length === 5 ? true : null,
      );
    },
    check: (page) => [
      ...wanted("the sessions found", texts(page, ".selector-row .selector-name"), [
        "run 102",
        "run 1020",
        "run 1021",
        "run 1022",
        "run 1023",
      ]),
      ...wanted("the rest", texts(page, ".selector-rows .selector-note"), []),
    ],
  },
  {
    // What a server's terminal prints reaches the clipboard only with the keyboard, its
    // links to files lead nowhere here, and a copy of its screen that cannot be shown is
    // said to be so in its pane.
    name: "served-terminals",
    window: SERVED,
    prepare: async (app, core) => {
      await until("the panes being attached", () => (core.attached() ? true : null));
      const inputOf = (pane: string) =>
        until("the pane's field", () =>
          document.querySelector<HTMLElement>(`.pane-card[data-pane="${pane}"] .terminal-input`),
        );
      const print = (pane: string, text: string) =>
        core.output(pane, new TextEncoder().encode(text));
      const copies = (pane: string, text: string) => print(pane, `\u001b]52;c;${btoa(text)}\u0007`);
      // A window that is not the system's own focus hears of none: it is told here.
      (await inputOf("p2")).dispatchEvent(new FocusEvent("blur"));
      (await inputOf("p1")).dispatchEvent(new FocusEvent("focus"));
      copies("p1", "with the keyboard");
      copies("p2", "without it");
      (await inputOf("p1")).dispatchEvent(new FocusEvent("blur"));
      copies("p1", "no longer with it");
      await until("the program's use of the clipboard being told", () => app.prompts.current());
      app.prompts.current()?.answer(false);
      if (JSON.stringify(core.copied) !== '["with the keyboard"]') {
        throw new Error(`what was copied: ${JSON.stringify(core.copied)}`);
      }

      print("p1", "\r\nfile:///C:/Windows https://example.test/\r\n");
      await until("the links being drawn", () =>
        app.views
          .get("p1")
          ?.screen()
          .lines.some((line) => line.includes("example.test"))
          ? true
          : null,
      );
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const cell = app.views.get("p1")?.cellRect(3, 1);
      const web = app.views.get("p1")?.cellRect(25, 1);
      const root = document.querySelector('.pane-card[data-pane="p1"] .terminal');
      if (!cell || !web || !root) throw new Error("the pane is not drawn");
      for (const at of [cell, web]) {
        root.dispatchEvent(
          new PointerEvent("pointerdown", {
            pointerId: 1,
            isPrimary: true,
            button: 0,
            buttons: 1,
            ctrlKey: true,
            metaKey: true,
            clientX: at.x + at.width / 2,
            clientY: at.y + at.height / 2,
            bubbles: true,
            cancelable: true,
          }),
        );
      }
      const told = await until("the link being refused", () => app.prompts.current());
      if (told.kind !== "notice") throw new Error(`a link to a file asked: ${told.kind}`);
      told.answer(false);
      await until("the other link being opened", () => (core.links.length > 0 ? true : null));
      if (JSON.stringify(core.links) !== '["https://example.test/"]') {
        throw new Error(`what was opened: ${JSON.stringify(core.links)}`);
      }
      print("p2", "still shown\r\n");
      core.copy("p2", "0.0.0-another", new Uint8Array([1, 2, 3]));
      await until("the copy being said not to be shown", () =>
        document.querySelector('.pane-card[data-pane="p2"] .terminal-status:not([hidden])'),
      );
    },
    check: (page) =>
      wanted("what is said of the copy", texts(page, ".terminal-status:not([hidden])"), [
        "What was on this terminal's screen before could not be shown.",
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
