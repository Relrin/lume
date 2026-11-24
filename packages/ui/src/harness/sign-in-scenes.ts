/**
 * The scenes of what the SSH client asks: a password, one that was not accepted, the
 * passphrase of a key and whether a machine is the one that is meant; and the question
 * over the Connect palette and over the dialog that adds a connection.
 */

import type { ShellApp } from "../shell/boot.ts";
import type { SignIn } from "../terminal/prompts.ts";
import type { FakeCoreOptions } from "./fake-core.ts";
import { FIXTURE_BINDINGS, FIXTURE_COMMANDS, HOST_COMMANDS } from "./fixture-commands.ts";
import type { Scene } from "./scene.ts";
import { CHROME_START_TIMEOUT_MS } from "./tuning.ts";

/** How often a scene looks again for what it waits for. */
const POLL_MS = 20;

const WINDOW = {
  tabs: [2],
  commands: [...FIXTURE_COMMANDS, ...HOST_COMMANDS],
  bindings: FIXTURE_BINDINGS,
} as const satisfies FakeCoreOptions;

const PASSWORD: SignIn = {
  ask: 1,
  kind: "password",
  host: "build-01",
  address: "ssh://mira@build-01.example",
  words: "mira@build-01.example's password:",
  again: false,
};

const HOST_KEY: SignIn = {
  ...PASSWORD,
  kind: "host_key",
  words:
    "The authenticity of host 'build-01.example (192.0.2.7)' can't be established.\n" +
    "ED25519 key fingerprint is SHA256:mVPwvezndPv/ARoIadVY98vAC0g+P/5633yTC4d/wXE.\n" +
    "This key is not known by any other names.\n" +
    "Are you sure you want to continue connecting (yes/no/[fingerprint])?",
};

async function until<T>(what: string, find: () => T | null | undefined | false): Promise<T> {
  const deadline = performance.now() + CHROME_START_TIMEOUT_MS;
  for (;;) {
    const got = find();
    if (got !== null && got !== undefined && got !== false) return got;
    if (performance.now() > deadline) throw new Error(`${what} never happened`);
    await new Promise((resolve) => window.setTimeout(resolve, POLL_MS));
  }
}

function question(): HTMLDialogElement | null {
  return document.querySelector<HTMLDialogElement>("dialog.sign-in[open]");
}

function field(): HTMLInputElement | null {
  return question()?.querySelector("input") ?? null;
}

function typeInto(input: HTMLInputElement, text: string): void {
  input.value = text;
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function press(key: string): void {
  const at = document.activeElement ?? document.body;
  at.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}

/** Puts a question, and gives what becomes of it once it has the keyboard. */
async function put(
  app: ShellApp,
  signIn: SignIn,
): Promise<{ readonly answered: Promise<string | null> }> {
  const answered = app.prompts.signIn(signIn);
  await until("the question having the keyboard", () =>
    question()?.contains(document.activeElement),
  );
  return { answered };
}

async function answers(what: string, answered: Promise<string | null>, should: string | null) {
  const answer = await answered;
  if (answer !== should) throw new Error(`${what} answered ${String(answer)}`);
  await until(`${what} going`, () => question() === null);
}

/** What the question says, in the order it is read. */
function said(page: Element): readonly string[] {
  return [...page.querySelectorAll("dialog.sign-in :is(h2, p, pre, .prompt-actions button)")].map(
    (element) => element.textContent ?? "",
  );
}

function wanted(what: string, got: unknown, want: unknown): string[] {
  const [left, right] = [JSON.stringify(got), JSON.stringify(want)];
  return left === right ? [] : [`${what} is ${left}, not ${right}`];
}

/** Whether the keyboard is where it is said to be, and cannot be taken from there. */
function keeps(what: string, element: Element | null | undefined): string[] {
  const problems = wanted(`${what} has the keyboard`, document.activeElement === element, true);
  for (const behind of document.querySelectorAll<HTMLElement>(
    ".terminal textarea, .top-bar button",
  )) {
    behind.focus();
    if (document.activeElement === behind) problems.push(`${what} lost the keyboard`);
  }
  return problems;
}

const FORGOTTEN = "Used for this connection, then forgotten.";

export const SIGN_IN_SCENES: readonly Scene[] = [
  {
    name: "ask-password",
    window: WINDOW,
    prepare: async (app) => {
      // Answered by Enter, and not by going ahead with nothing typed.
      const first = await put(app, PASSWORD);
      await until("the field", field);
      press("Enter");
      if (!question()) throw new Error("nothing was taken for a password");
      const input = field();
      if (input) typeInto(input, "correct horse");
      press("Enter");
      await answers("the password", first.answered, "correct horse");
      // Put away by Escape.
      const second = await put(app, PASSWORD);
      await until("the field", field);
      press("Escape");
      await answers("Escape", second.answered, null);
      // Taken back, which is no answer.
      const third = await put(app, { ...PASSWORD, ask: 9 });
      await until("the field", field);
      app.prompts.withdraw(9);
      await answers("the question that was taken back", third.answered, null);
      await put(app, PASSWORD);
      const last = await until("the field", field);
      if (last.value !== "") throw new Error("the field began with something in it");
      typeInto(last, "correct horse");
      await until("it being something to go ahead with", () =>
        question()?.querySelector(".prompt-proceed:not(:disabled)"),
      );
    },
    check: (page) => [
      ...wanted("what is said", said(page), [
        "Sign in to build-01",
        PASSWORD.address,
        PASSWORD.words,
        FORGOTTEN,
        "Cancel",
        "Sign in",
      ]),
      ...wanted("the field", field()?.type, "password"),
      ...keeps("the field", field()),
    ],
  },
  {
    name: "ask-password-again",
    window: WINDOW,
    prepare: async (app) => {
      await put(app, { ...PASSWORD, again: true });
      const input = await until("the field", field);
      typeInto(input, "correct horse");
      question()?.querySelector<HTMLButtonElement>(".sign-in-field button")?.click();
      await until("what is typed being shown", () => field()?.type === "text");
    },
    check: (page) => [
      ...wanted("what is said", said(page)[3], "That was not accepted."),
      ...keeps("the field", field()),
    ],
  },
  {
    name: "ask-passphrase",
    window: WINDOW,
    prepare: async (app) => {
      const passphrase: SignIn = {
        ...PASSWORD,
        kind: "passphrase",
        words: "Enter passphrase for key '/home/mira/.ssh/id_ed25519':",
      };
      const skipped = await put(app, passphrase);
      await until("the field", field);
      [...(question()?.querySelectorAll<HTMLButtonElement>(".prompt-actions button") ?? [])]
        .find((button) => button.textContent === "Skip this key")
        ?.click();
      await answers("skipping the key", skipped.answered, "");
      await put(app, passphrase);
      await until("the field", field);
    },
    check: (page) => [
      ...wanted("the answers", said(page).slice(-3), ["Cancel", "Skip this key", "Unlock"]),
      ...keeps("the field", field()),
    ],
  },
  {
    name: "ask-host-key",
    window: WINDOW,
    prepare: async (app) => {
      const trusted = await put(app, HOST_KEY);
      const button = await until("the answer", () =>
        question()?.querySelector<HTMLButtonElement>(".prompt-proceed"),
      );
      button.click();
      await answers("trusting the machine", trusted.answered, "yes");
      await put(app, HOST_KEY);
      await until("the question", question);
    },
    check: (page) => [
      ...wanted("the answers", said(page).slice(-2), ["Cancel", "Trust and connect"]),
      ...keeps("the cautious answer", question()?.querySelector(".prompt-actions button")),
    ],
  },
  {
    // The palette stays on show beneath, and has the keyboard again afterwards.
    name: "ask-over-connect",
    window: WINDOW,
    prepare: async (app) => {
      app.connect.getState().open("all");
      await until("the palette", () => document.querySelector(".connect-input"));
      const before = document.activeElement;
      const asked = await put(app, PASSWORD);
      await until("the field", field);
      press("Escape");
      await answers("Escape", asked.answered, null);
      if (!app.connect.getState().shown) throw new Error("the palette went with the question");
      await until("the palette having the keyboard again", () => document.activeElement === before);
      await put(app, PASSWORD);
      await until("the field", field);
    },
    check: () => keeps("the field", field()),
  },
  {
    // So does the dialog that adds a connection, with what is typed in it.
    name: "ask-over-connection",
    window: WINDOW,
    prepare: async (app) => {
      app.commands.run("lume.connection.add", { kind: "rex" });
      await until("the dialog", () => document.querySelector(".connection [data-first]"));
      const before = document.activeElement;
      const asked = await put(app, PASSWORD);
      await until("the field", field);
      press("Escape");
      await answers("Escape", asked.answered, null);
      if (!document.querySelector(".connection")) {
        throw new Error("the dialog went with the question");
      }
      await until("the dialog having the keyboard again", () => document.activeElement === before);
      await put(app, PASSWORD);
      await until("the field", field);
    },
    check: () => keeps("the field", field()),
  },
];
