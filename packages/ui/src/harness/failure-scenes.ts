/**
 * The scenes of what cannot be drawn, and of what is drawn of a server that means harm.
 *
 * A part of the page is made to fail by giving it something it was never meant to be
 * given, where the page keeps what it heard from the core. The core gives no such thing:
 * what is looked at is what becomes of the rest of the window.
 */

import type { RexHost } from "../core/types.ts";
import type { ShellApp } from "../shell/boot/index.ts";
import type { FakeCore, FakeCoreOptions } from "./fake-core.ts";
import {
  texts,
  typeInto,
  until,
  wanted,
  CI,
  DEVBOX,
  LAB,
  NAS,
  SERVED,
  WINDOW,
} from "./rex-scenes.ts";
import type { Scene } from "./scene.ts";

/** A server nothing can be said of. */
const UNSAYABLE = { ...DEVBOX, on_show: "session:1", state: undefined } as unknown as RexHost;

function said(what: string, is: unknown, should: unknown): void {
  const problems = wanted(what, is, should);
  if (problems.length > 0) throw new Error(problems.join("; "));
}

/** Puts servers in the page's store, past what the core would send. */
function setServers(app: ShellApp, hosts: readonly RexHost[]): void {
  const revision = (app.rex.getState().list?.revision ?? 0) + 1;
  app.rex.setState({ list: { revision, local: true, hosts: [...hosts] } });
}

function print(core: FakeCore, pane: string, text: string): void {
  core.output(pane, new TextEncoder().encode(text));
}

async function shows(app: ShellApp, pane: string, text: string): Promise<void> {
  await until(`${pane} showing “${text}”`, () =>
    app.views
      .get(pane)
      ?.screen()
      .lines.some((line) => line.includes(text))
      ? true
      : null,
  );
}

const failures = () => texts(document, ".part-failed-text");
const notices = () => texts(document, ".notice-message");

/** Names that say something else than they seem to, or more than there is room for. */
const NAMES = [
  "production\u202egnp.exe",
  "عمل\u200e / خادم",
  "e" + "\u0336\u0327".repeat(60),
  "<img src=x onerror=alert(1)>",
  "constructor",
  "\u200b\u2060\u3164",
  "name ".repeat(204).trim(),
  "line one\nline two",
  "👩\u200d💻".repeat(80),
  "Attached",
];

/** A window of a server whose sessions are called so, and which is called so itself. */
const HOSTILE = {
  ...SERVED,
  rexHosts: {
    local: true,
    hosts: [
      {
        ...DEVBOX,
        label: "خادم " + "long ".repeat(200).trim(),
        sessions: NAMES.map((label, at) => ({ id: `session:${at}`, label })),
        on_show: "session:1",
        state: {
          kind: "connected",
          version: "0".repeat(1024),
          platform: "constructor_amd64",
          api_revision: 19,
        },
      },
      { ...NAS, label: "__proto__" },
      {
        ...CI,
        label: "failed",
        state: { kind: "failed", code: "constructor", error: "why ".repeat(250).trim() },
      },
      LAB,
    ],
  },
} as const satisfies FakeCoreOptions;

/** What an element's content reaches past its edge by, sideways. */
function spills(selector: string): string[] {
  return [...document.querySelectorAll<HTMLElement>(selector)]
    .filter((box) => box.scrollWidth > box.clientWidth)
    .map((box) => `${selector} is ${box.scrollWidth} wide in ${box.clientWidth}`);
}

/** The scenes of what fails, in the order they are run. */
export const FAILURE_SCENES: readonly Scene[] = [
  {
    // The bars that name the server cannot be drawn, nor can one of the panes. The rest of
    // the window goes on, and what failed is there again when it can be drawn.
    name: "view-failed",
    window: SERVED,
    prepare: async (app, core) => {
      await until("the panes being attached", () => (core.attached() ? true : null));
      const good = await until("the servers arriving", () => app.rex.getState().list);
      const field = document.querySelector<HTMLElement>(
        '.pane-card[data-pane="p1"] .terminal-input',
      );
      setServers(app, [UNSAYABLE, NAS]);
      await until("the bars failing", () => (failures().length === 2 ? true : null));
      print(core, "p1", "still drawn\r\n");
      await shows(app, "p1", "still drawn");
      // It is the same card, with the same terminal in it.
      said(
        "the terminal's field",
        document.querySelector('.pane-card[data-pane="p1"] .terminal-input') === field,
        true,
      );
      // Asked for while nothing has changed, they fail again, and nothing else does.
      document.querySelector<HTMLButtonElement>(".top-left .part-failed-again")?.click();
      await until("the bars failing again", () => (failures().length === 2 ? true : null));
      // By themselves, once the server can be named again.
      setServers(app, good.hosts);
      await until("the bars being drawn again", () =>
        failures().length === 0 && document.querySelector(".switcher-name") ? true : null,
      );
      said("the status bar", texts(document, ".status-served-text").length, 1);

      // A header alone, with the terminal under it as it was.
      app.status
        .getState()
        .register({ id: "bad", kind: "text", slot: "pane.header", pane: "p1", text: {} } as never);
      await until("the header failing", () =>
        failures().join() === "This header could not be drawn." ? true : null,
      );
      said(
        "the terminal's field",
        document.querySelector('.pane-card[data-pane="p1"] .terminal-input') === field,
        true,
      );
      app.status.getState().remove("bad");
      await until("the header being drawn again", () => (failures().length === 0 ? true : null));

      const pane = app.layout.getState().panes["p2"];
      if (!pane) throw new Error("there is no p2");
      const broken = { type: "placeholder", label: {}, kind: "" } as never;
      app.layout.setState({
        panes: { ...app.layout.getState().panes, p2: { ...pane, content: broken } },
      });
      await until("the pane failing", () =>
        failures()[0] === "This pane could not be drawn." ? true : null,
      );
      print(core, "p1", "and still\r\n");
      await shows(app, "p1", "and still");
      setServers(app, [UNSAYABLE, NAS]);
      await until("all three failing", () => (failures().length === 3 ? true : null));
    },
    check: (page) => [
      ...wanted("what failed", texts(page, ".part-failed-text"), [
        "What is on show could not be drawn.",
        "This pane could not be drawn.",
        "The status bar could not be drawn.",
      ]),
      ...wanted("the panes", page.querySelectorAll(".pane-card").length, 2),
      ...wanted("the tabs", page.querySelectorAll('[role="tab"]').length, 1),
    ],
  },
  {
    // What opens over the window and cannot be drawn is put away, the keyboard goes back
    // to the terminal, and a question nobody could see is answered no.
    name: "overlay-failed",
    window: WINDOW,
    prepare: async (app) => {
      const good = await until("the servers arriving", () => app.rex.getState().list);
      const back = () =>
        until("the keyboard going back", () =>
          document.activeElement?.classList.contains("terminal-input") ? true : null,
        );
      const tells = (what: string) =>
        until(`“${what}” being said`, () => (notices().includes(what) ? true : null));
      setServers(app, [UNSAYABLE]);

      app.palette.getState().open();
      await tells("The command palette could not be shown.");
      said("the palette", app.palette.getState().shown, false);
      await back();
      app.connect.getState().open("rex");
      await tells("The Connect palette could not be shown.");
      said("the Connect palette", app.connect.getState().shown, false);
      await back();
      document.querySelector<HTMLButtonElement>(".switcher-button")?.click();
      await tells("The list of hosts and sessions could not be shown.");
      await until("the selector going", () =>
        document.querySelector('.switcher-button[aria-expanded="false"]'),
      );
      await back();

      // Once it can be drawn, what is asked for opens.
      setServers(app, good.hosts);
      app.palette.getState().open();
      await until("the palette opening", () => document.querySelector(".palette-input"));
      app.palette.getState().close();
      await back();

      // Two questions that cannot be drawn and one that can, asked at once.
      const bad = { title: "End?", message: {}, confirm: "End" } as never;
      const answers = [app.prompts.confirm(bad), app.prompts.confirm(bad)];
      const last = app.prompts.confirm({ title: "End this?", message: "It ends.", confirm: "End" });
      said("the answers", await Promise.all(answers), [false, false]);
      await tells("A question could not be shown, and was answered no.");
      await until("the next question", () =>
        document.querySelector('.prompt[aria-label="End this?"]'),
      );
      app.prompts.current()?.answer(true);
      said("its answer", await last, true);
      await until("the question going", () => (document.querySelector(".prompt") ? null : true));
      app.views.focus();
      await back();
    },
    check: (page) => [
      ...wanted("what failed", texts(page, ".part-failed-text"), []),
      ...wanted("what is open", page.querySelectorAll(".palette, .connect, .prompt").length, 0),
    ],
  },
  {
    // Nothing of the window can be drawn: it says why, and its own buttons are there.
    name: "window-failed",
    window: { ...WINDOW, titleBar: "custom" },
    prepare: async (app, core) => {
      await until("the panes being attached", () => (core.attached() ? true : null));
      const good = await until("the configuration", () => app.config.getState().view);
      const appearance = Object.defineProperty({ ...good.config.appearance }, "tile-color", {
        get: () => {
          throw new Error("the colour of the tiles is nowhere to be found");
        },
      });
      const bad = { ...good, config: { ...good.config, appearance } };
      const fail = async () => {
        app.config.setState({ view: bad });
        await until("the window failing", () => document.querySelector(".window-failed"));
      };
      await fail();
      // The terminals are there all the while.
      print(core, "p1", "printed meanwhile\r\n");
      await shows(app, "p1", "printed meanwhile");
      const again = () =>
        document.querySelector<HTMLButtonElement>(".window-failed-ways button")?.click();
      again();
      await until("it failing again", () => document.querySelector(".window-failed"));
      app.config.setState({ view: good });
      again();
      await until("the panes being drawn again", () =>
        document.querySelectorAll(".pane-card canvas").length === 2 ? true : null,
      );
      await shows(app, "p1", "printed meanwhile");
      print(core, "p1", "printed since\r\n");
      await shows(app, "p1", "printed since");
      await fail();
    },
    check: (page) => [
      ...wanted("what is said", texts(page, ".start-failure h1, .start-failure p"), [
        "The window could not be drawn",
        "the colour of the tiles is nowhere to be found",
      ]),
      ...wanted("the window's buttons", page.querySelectorAll(".window-control").length, 3),
    ],
  },
  {
    // A server that calls itself and its sessions whatever it likes: in the palettes, and
    // in the selector, which is left on show.
    name: "hostile-names",
    window: HOSTILE,
    prepare: async (app) => {
      await until("the servers arriving", () => app.rex.getState().list);
      const spilt = () => [
        ...spills(".palette"),
        ...spills(".connect"),
        ...spills(".selector"),
        ...spills(".top-bar"),
        ...spills(".status-bar"),
        ...[...document.querySelectorAll('img[src="x"]')].map(() => "a name was read as markup"),
      ];
      app.palette.getState().open();
      const palette = await until("the palette's field", () =>
        document.querySelector<HTMLInputElement>(".palette-input"),
      );
      typeInto(palette, "#");
      await until("the hosts being listed", () =>
        document.querySelectorAll(".palette-row").length > NAMES.length ? true : null,
      );
      const shown = texts(document, ".palette-row-text").slice(1, NAMES.length + 1);
      said(
        "the first names",
        [shown[0], shown[3], shown[4], shown[5], shown[7]],
        [
          "productiongnp.exe",
          "<img src=x onerror=alert(1)>",
          "constructor",
          "unnamed",
          "line one line two",
        ],
      );
      said("the marks on one letter", [...(shown[2] ?? "")].length, 5);
      said("what the palette spills", spilt(), []);
      app.palette.getState().close();

      app.connect.getState().open("rex");
      await until("the sessions being listed", () =>
        document.querySelectorAll(".connect-session").length === NAMES.length ? true : null,
      );
      said("the system", texts(document, ".connect-stats dd")[1], "constructor");
      said("how the servers stand", texts(document, ".connect-row-line").slice(1, 3), [
        "Not connected",
        "Failed",
      ]);
      said("what the Connect palette spills", spilt(), []);
      app.connect.getState().close();
      await until("it going", () => (document.querySelector(".connect") ? null : true));

      document.querySelector<HTMLButtonElement>(".switcher-button")?.click();
      await until("the sessions being listed", () =>
        document.querySelectorAll(".selector-row").length === NAMES.length ? true : null,
      );
      said("what the selector spills", spilt(), []);
    },
    check: (page) =>
      wanted("what is marked as on show", texts(page, ".selector-row[data-on] .selector-right"), [
        "Attached",
      ]),
  },
  {
    // The question about ending such a session, in the words the core puts it in.
    name: "hostile-question",
    window: HOSTILE,
    prepare: async (app) => {
      await until("the servers arriving", () => app.rex.getState().list);
      void app.prompts.confirm({
        title: `End “${"name ".repeat(150)}${"W".repeat(274)}”?`,
        message:
          "The session is ended on خادم لينكس, with every program in it, for everyone who is " +
          "connected to it.",
        confirm: "End session",
      });
      await until("the question", () => document.querySelector(".prompt"));
    },
    check: (page) => {
      const box = page.querySelector(".prompt")?.getBoundingClientRect();
      const title = page.querySelector(".prompt h2");
      const go = page.querySelector(".prompt-proceed")?.getBoundingClientRect();
      const bounds = page.getBoundingClientRect();
      return [
        ...wanted("that the question is in the window", !!box && box.bottom <= bounds.bottom, true),
        ...wanted("that its buttons are", !!go && go.bottom <= bounds.bottom && go.top >= 0, true),
        ...wanted("that its title leaves room", (title?.clientHeight ?? 999) <= 130, true),
        ...spills(".prompt"),
        ...spills(".prompt h2"),
        ...wanted(
          "what has the keyboard",
          page.querySelector(".prompt-actions button") === document.activeElement,
          true,
        ),
      ];
    },
  },
];
