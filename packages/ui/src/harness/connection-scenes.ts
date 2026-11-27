/**
 * The scenes of the dialog that adds and changes connections: an address as it is read,
 * a test that went well and one that did not, a machine reached over SSH, the Linux
 * distributions, and the dialog over the settings window.
 */

import { connectionDialog } from "../connections/store.ts";
import type { Profile, ProfileList } from "../core/types.ts";
import type { ShellApp } from "../shell/boot/index.ts";
import type { FakeCoreOptions } from "./fake-core.ts";
import { FIXTURE_BINDINGS, FIXTURE_COMMANDS, HOST_COMMANDS } from "./fixture-commands.ts";
import type { Scene } from "./scene.ts";
import { CHROME_START_TIMEOUT_MS } from "./tuning.ts";

/** How often a scene looks again for what it waits for. */
const POLL_MS = 20;

function found(id: string, name: string, launcher: Profile["launcher"], detail: string): Profile {
  return { id, name, launcher, hidden: false, origin: "generated", detail };
}

const SSH_HOST = "from ~/.ssh/config";

const PROFILES: ProfileList = {
  revision: 1,
  default: null,
  notes: [],
  looked: true,
  looks: {},
  profiles: [
    {
      id: "gpu",
      name: "GPU box",
      launcher: {
        type: "ssh",
        host: "10.0.3.21",
        user: "mira",
        port: 2222,
        extra_args: ["-i", "~/.ssh/gpu"],
      },
      hidden: false,
      origin: "user",
    },
    found("auto.wsl.Ubuntu", "Ubuntu", { type: "wsl", distro: "Ubuntu" }, "WSL 2 · running"),
    {
      ...found("auto.wsl.Debian", "Debian", { type: "wsl", distro: "Debian" }, "WSL 2 · stopped"),
      hidden: true,
    },
    found("auto.ssh.build", "build", { type: "ssh", host: "build" }, SSH_HOST),
    found("auto.ssh.devbox", "devbox", { type: "ssh", host: "devbox" }, SSH_HOST),
  ],
};

const DEVBOX = {
  id: "rex:ssh://devbox",
  label: "devbox-01",
  address: "ssh://devbox",
  local: false,
  written: true,
  state: {
    kind: "connected",
    version: "0.1.0-preview.22",
    platform: "linux_amd64",
    api_revision: 19,
  },
  sessions: [{ id: "session:1", label: "acme" }],
  on_show: null,
  latency_ms: 12,
} as const;

/** What the settings window's page of hosts is shown with. */
export const HOSTS_WINDOW: FakeCoreOptions = {
  titleBar: "custom",
  profiles: PROFILES,
  rexHosts: {
    local: true,
    hosts: [
      {
        ...DEVBOX,
        id: "rex:local",
        label: "Local Rex",
        address: "local",
        local: true,
        written: false,
        state: { kind: "absent" },
        sessions: [],
        latency_ms: null,
      },
      DEVBOX,
      {
        ...DEVBOX,
        id: "rex:https://home-nas.example",
        label: "home-nas",
        address: "https://home-nas.example",
        state: { kind: "idle" },
        sessions: [],
        latency_ms: null,
      },
    ],
  },
};

const WINDOW = {
  tabs: [2],
  commands: [...FIXTURE_COMMANDS, ...HOST_COMMANDS],
  bindings: FIXTURE_BINDINGS,
  profiles: PROFILES,
  rexHosts: HOSTS_WINDOW.rexHosts ?? { local: true, hosts: [] },
} as const satisfies FakeCoreOptions;

async function until<T>(what: string, find: () => T | null | undefined | false): Promise<T> {
  const deadline = performance.now() + CHROME_START_TIMEOUT_MS;
  for (;;) {
    const got = find();
    if (got !== null && got !== undefined && got !== false) return got;
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

function button(label: string): HTMLButtonElement | undefined {
  return [...document.querySelectorAll<HTMLButtonElement>(".connection-button")].find(
    (one) => one.textContent === label,
  );
}

async function pressButton(label: string): Promise<void> {
  const one = await until(`“${label}” being offered`, () => {
    const offered = button(label);
    return offered && !offered.disabled ? offered : null;
  });
  one.click();
}

/** Opens the dialog through its command, and types the address into it. */
async function addRex(app: ShellApp, address: string): Promise<void> {
  await until("the servers arriving", () => app.rex.getState().list);
  await until("the profiles arriving", () => app.profiles.getState().list);
  app.commands.run("lume.connection.add", { kind: "rex" });
  const field = await until("the dialog opening", () =>
    document.querySelector<HTMLInputElement>("#connection-address"),
  );
  if (address === "") return;
  typeInto(field, address);
  await until("the address being read", () =>
    document.querySelector('.connection-read:not([data-tone="idle"])'),
  );
}

/** Opens the dialog on WSL through its command, and picks a distribution and a folder. */
async function addWsl(app: ShellApp): Promise<void> {
  await until("the profiles arriving", () => app.profiles.getState().list);
  app.commands.run("lume.connection.add", { kind: "wsl" });
  const distro = await until("the dialog opening", () =>
    document.querySelector<HTMLInputElement>("#connection-distro"),
  );
  typeInto(distro, "Ubuntu");
  const cwd = await until("the folder field", () =>
    document.querySelector<HTMLInputElement>("#connection-cwd"),
  );
  typeInto(cwd, "~/code");
}

function tested(state: "ok" | "bad"): Promise<Element> {
  return until("the test ending", () =>
    state === "ok"
      ? [...document.querySelectorAll(".connection-step")].every(
          (step) => step.getAttribute("data-state") === "ok",
        ) && document.querySelector(".connection-step")
      : document.querySelector('.connection-step[data-state="bad"]'),
  );
}

function texts(page: ParentNode, selector: string): string[] {
  return [...page.querySelectorAll(selector)].map((one) => one.textContent ?? "");
}

function wanted(what: string, is: unknown, should: unknown): string[] {
  return JSON.stringify(is) === JSON.stringify(should)
    ? []
    : [`${what}: ${JSON.stringify(is)}, not ${JSON.stringify(should)}`];
}

/** The scenes of the dialog, in the order they are run. */
export const CONNECTION_SCENES: readonly Scene[] = [
  {
    // As it opens: nothing typed, and what the user's SSH configuration has to offer.
    name: "connection",
    window: WINDOW,
    prepare: (app) => addRex(app, ""),
    check: (page) => [
      ...wanted("the hosts offered", texts(page, ".connection-chip"), ["build", "devboxadded"]),
      ...wanted("beside the buttons", texts(page, ".connection-status"), ["Testing is optional"]),
      ...wanted("whether it can be added", button("Add connection")?.disabled, true),
      ...(page.querySelector("#connection-address") === document.activeElement
        ? []
        : ["the address does not have the keyboard"]),
    ],
  },
  {
    name: "connection-tested",
    window: WINDOW,
    prepare: async (app) => {
      await addRex(app, "ssh://alice@build.example.com");
      await pressButton("Test");
      await tested("ok");
    },
    check: (page) => [
      ...wanted("beside the buttons", texts(page, ".connection-status"), ["Connection works"]),
      ...wanted("what the steps say", texts(page, ".connection-step em"), [
        "12 ms",
        "",
        "rex 0.1.0-preview.22 · Linux",
      ]),
      ...wanted(
        "what it would be called",
        page.querySelector<HTMLInputElement>("#connection-name")?.placeholder,
        "build",
      ),
      ...wanted("whether it can be added", button("Add connection")?.disabled, false),
    ],
  },
  {
    // A machine without Rex: what can be done instead.
    name: "connection-failed",
    window: WINDOW,
    prepare: async (app) => {
      await addRex(app, "ssh://old.example.com");
      await pressButton("Test");
      await tested("bad");
    },
    check: (page) => [
      ...wanted("beside the buttons", texts(page, ".connection-status"), [
        "Rex isn't installed there",
      ]),
      ...wanted("what can be done", texts(page, ".connection-fix button"), ["Use SSH instead"]),
      ...wanted("whether it can be added", button("Add connection")?.disabled, true),
    ],
  },
  {
    // Nothing between the two ends would be encrypted, and the test says so first.
    name: "connection-unencrypted",
    window: WINDOW,
    prepare: async (app) => {
      await addRex(app, "http://192.0.2.7:7297");
      await pressButton("Test");
      await tested("bad");
    },
    check: (page) => [
      ...wanted("what can be done", texts(page, ".connection-fix button"), ["Test anyway"]),
      ...wanted("whether it can be added", button("Add connection")?.disabled, false),
    ],
  },
  {
    // A profile of the user's own being changed: its kind stays what it is.
    name: "connection-ssh",
    window: WINDOW,
    prepare: async (app) => {
      const list = await until("the profiles arriving", () => app.profiles.getState().list);
      const profile = list.profiles[0];
      if (!profile) throw new Error("there is no profile to change");
      connectionDialog.getState().edit({ kind: "ssh", profile }, true);
      await until("the dialog opening", () => document.querySelector("#connection-host"));
    },
    check: (page) => [
      ...wanted("what it is called", texts(page, ".connection-head h2"), ["Edit GPU box"]),
      ...wanted(
        "the key",
        page.querySelector<HTMLInputElement>('[aria-label="Key file"]')?.value,
        "~/.ssh/gpu",
      ),
      ...wanted("whether it can be saved", button("Save")?.disabled, false),
    ],
  },
  {
    // A distribution picked and a folder to start in: the name is the distribution's.
    name: "connection-wsl",
    window: WINDOW,
    prepare: async (app, core) => {
      await addWsl(app);
      await until("it being something to add", () => button("Add connection")?.disabled === false);
      if (core.written.length > 0) throw new Error("it was written before it was added");
    },
    check: (page) => [
      ...wanted(
        "the distributions offered",
        [...page.querySelectorAll("#connection-distro-list option")].map(
          (one) => (one as HTMLOptionElement).value,
        ),
        ["Ubuntu", "Debian"],
      ),
      ...wanted(
        "what it is called by default",
        page.querySelector<HTMLInputElement>("#connection-name")?.placeholder,
        "Ubuntu",
      ),
      ...wanted("the distributions found", texts(page, ".connection-distro-name"), [
        "UbuntuWSL 2 · running",
        "DebianWSL 2 · stopped",
      ]),
      ...wanted("beside the buttons", texts(page, ".connection-status"), ["1 of 2 listed"]),
    ],
  },
  {
    // An added distribution is what the Connect palette shows next, among those of WSL.
    name: "connection-wsl-added",
    window: WINDOW,
    prepare: async (app, core) => {
      await addWsl(app);
      await pressButton("Add connection");
      await until("the dialog going", () => !document.querySelector(".connection"));
      const wrote = JSON.stringify(core.written);
      const should =
        '"id":null,"name":"Ubuntu","wsl":{"distro":"Ubuntu","user":null,"cwd":"~/code"}';
      if (core.written.length !== 1 || !wrote.includes(should)) {
        throw new Error(`what was written: ${wrote}`);
      }
      await until("the palette listing it", () =>
        [...document.querySelectorAll(".connect-row-line")].find(
          (line) => line.textContent === "WSL · Ubuntu",
        ),
      );
    },
    check: (page) => [
      ...wanted("the palette listing", texts(page, ".connect-row-line"), [
        "WSL · Ubuntu",
        "WSL 2 · running",
      ]),
    ],
  },
  {
    // A server that was added is what the selector shows next.
    name: "connection-added",
    window: WINDOW,
    prepare: async (app, core) => {
      await addRex(app, "nas.lan");
      await pressButton("Add connection");
      await until("the selector showing it", () =>
        [...document.querySelectorAll('.selector-host[aria-selected="true"] .selector-name')].find(
          (name) => name.textContent === "nas",
        ),
      );
      const wrote = JSON.stringify(core.written);
      const should = '"was":null,"label":"nas","address":"nas.lan"';
      if (core.written.length !== 1 || !wrote.includes(should)) {
        throw new Error(`what was written: ${wrote}`);
      }
    },
    check: (page) => [
      ...wanted("what can be done", texts(page, ".selector-side .selector-act-label"), [
        "Connect",
        "Edit connection…",
      ]),
      ...wanted("below the hosts", texts(page, ".selector-add .selector-act-label"), [
        "Add connection…",
      ]),
    ],
  },
];
