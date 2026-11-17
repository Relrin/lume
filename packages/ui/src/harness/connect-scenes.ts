/**
 * The scenes of the palette that opens a terminal from a profile, and of the menu
 * beside the new-tab button: the palette as it opens, the palette narrowed by a filter
 * and a query, and the menu open.
 */

import type { ProfileList } from "../core/types.ts";
import type { ShellApp } from "../shell/boot.ts";
import { FIXTURE_BINDINGS, FIXTURE_COMMANDS } from "./fixture-commands.ts";
import type { Scene } from "./scene.ts";
import { CHROME_START_TIMEOUT_MS } from "./tuning.ts";

/** How often a scene looks again for what it waits for. */
const POLL_MS = 20;

/** The profiles of the scenes: some of the user's own, some found, one hidden, a note. */
const SCENE_PROFILES: ProfileList = {
  revision: 1,
  default: "auto.pwsh",
  looked: true,
  notes: ["profile `work`: it says neither `program`, `distro` nor `host`"],
  profiles: [
    {
      id: "box",
      name: "Build box",
      launcher: { type: "ssh", host: "10.0.4.21", user: "mira", port: 2222 },
      cwd: "/srv/build",
      hidden: false,
      origin: "user",
    },
    {
      id: "auto.pwsh",
      name: "PowerShell",
      launcher: {
        type: "shell",
        program: "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
        args: ["-NoLogo"],
        login: false,
      },
      hidden: false,
      origin: "generated",
    },
    {
      id: "auto.cmd",
      name: "Command Prompt",
      launcher: {
        type: "shell",
        program: "C:\\Windows\\System32\\cmd.exe",
        args: [],
        login: false,
      },
      hidden: true,
      origin: "generated",
    },
    {
      id: "auto.git-bash",
      name: "Git Bash",
      launcher: {
        type: "shell",
        program: "C:\\Program Files\\Git\\bin\\bash.exe",
        args: ["--login", "-i"],
        login: false,
      },
      hidden: false,
      origin: "generated",
    },
    {
      id: "auto.wsl.Ubuntu",
      name: "Ubuntu",
      launcher: { type: "wsl", distro: "Ubuntu" },
      hidden: false,
      origin: "generated",
      detail: "WSL 2 · running",
    },
    {
      id: "auto.wsl.Debian",
      name: "Debian",
      launcher: { type: "wsl", distro: "Debian" },
      hidden: false,
      origin: "generated",
      detail: "WSL 2 · stopped",
    },
    {
      id: "auto.ssh.dev-staging",
      name: "dev-staging",
      launcher: { type: "ssh", host: "dev-staging" },
      hidden: false,
      origin: "generated",
      detail: "from ~/.ssh/config",
    },
  ],
};

const WINDOW = {
  tabs: [2],
  commands: FIXTURE_COMMANDS,
  bindings: FIXTURE_BINDINGS,
  profiles: SCENE_PROFILES,
} as const;

async function until<T>(what: string, find: () => T | null | undefined): Promise<T> {
  const deadline = performance.now() + CHROME_START_TIMEOUT_MS;
  for (;;) {
    const found = find();
    if (found !== null && found !== undefined) return found;
    if (performance.now() > deadline) throw new Error(`${what} never happened`);
    await new Promise((resolve) => window.setTimeout(resolve, POLL_MS));
  }
}

/** Opens the Connect palette once the page has the profiles, and waits for it. */
async function openConnect(app: ShellApp): Promise<HTMLElement> {
  await until("the profiles arriving", () => app.profiles.getState().list);
  app.connect.getState().open();
  return until("the Connect palette opening", () =>
    document.querySelector<HTMLElement>(".connect"),
  );
}

/** Types into a field the way a user would, so that the component hears of it. */
function typeInto(field: HTMLInputElement, text: string): void {
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  set?.call(field, text);
  field.dispatchEvent(new Event("input", { bubbles: true }));
}

/** The scenes of the Connect palette and the new-tab menu, in the order they are run. */
export const CONNECT_SCENES: readonly Scene[] = [
  {
    name: "connect",
    window: WINDOW,
    prepare: async (app) => {
      await openConnect(app);
    },
    check: (page) => {
      const problems: string[] = [];
      const rows = page.querySelectorAll(".connect-row");
      // Six profiles are visible; the hidden one is not listed.
      if (rows.length !== 6) problems.push(`the palette lists ${rows.length} profiles, not 6`);
      if (page.querySelector(".connect-input") !== document.activeElement) {
        problems.push("the palette's field does not have the keyboard");
      }
      return problems;
    },
  },
  {
    name: "connect-filter",
    window: WINDOW,
    prepare: async (app) => {
      const palette = await openConnect(app);
      palette.querySelector<HTMLButtonElement>('.connect-chip[data-filter="wsl"]')?.click();
      const field = palette.querySelector<HTMLInputElement>(".connect-input");
      if (field) typeInto(field, "deb");
      await until("the list narrowing", () =>
        palette.querySelectorAll(".connect-row").length === 1 ? true : null,
      );
    },
    check: (page) => {
      const name = page.querySelector(".connect-row .connect-row-name")?.textContent ?? "";
      return name === "Debian" ? [] : [`the palette marks ${JSON.stringify(name)}, not Debian`];
    },
  },
  {
    name: "app-menu-new-tab-with",
    window: WINDOW,
    prepare: async (app) => {
      await until("the profiles arriving", () => app.profiles.getState().list);
      const button = await until("the application's menu button", () =>
        document.querySelector<HTMLButtonElement>(".app-menu-button"),
      );
      button.click();
      const item = await until("the item that opens a tab with a profile", () =>
        document.querySelector<HTMLButtonElement>('.app-menu-drop [data-key="new-tab-with"]'),
      );
      // From the keyboard, as a person without a pointer opens it: the right arrow on
      // the item opens its menu, and the keyboard goes into it.
      item.focus();
      item.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
      await until("its menu opening with the keyboard in it", () =>
        document.activeElement?.closest(".menu-sub-place") ? true : null,
      );
    },
    check: (page) => {
      const items = [...page.querySelectorAll(".menu-sub-place .menu-item")].map(
        (item) => item.querySelector(".menu-label")?.textContent ?? "",
      );
      const wanted = [
        "Default terminal",
        "Build box",
        "Git Bash",
        "Ubuntu",
        "Debian",
        "dev-staging",
        "More…",
      ];
      return JSON.stringify(items) === JSON.stringify(wanted)
        ? []
        : [`the menu of "New tab with" lists ${JSON.stringify(items)}`];
    },
  },
];
