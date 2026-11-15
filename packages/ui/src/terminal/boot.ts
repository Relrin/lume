/**
 * Starting the terminal: once for each page, however often the component mounts.
 *
 * What is slow or can be had only once is done here: loading the terminal engine, asking
 * for a graphics device, opening the carrier. In development React mounts components
 * twice, and a second carrier would retire the first, so none of this may follow the
 * component's life.
 *
 * The window holds one session at a time on screen. A session that is started while
 * another is shown takes its place; the other keeps running, and is still there when
 * the page is reloaded.
 */

import { RenderHost } from "@lume/renderer";
import { KeyEncoder, Vt } from "@lume/vt";
import wasmUrl from "@lume/vt/ghostty-vt.wasm?url";

import { checkFrameContract } from "../frame-contract.ts";
import { DEFAULT_APPEARANCE } from "../theme.ts";
import { TauriCarrier } from "../transport/carrier.ts";
import { TransportClient } from "../transport/client.ts";
import { CommandError } from "../transport/types.ts";
import { currentPlatform, KeyLayout } from "./keyboard.ts";
import { PromptStore } from "./prompts.ts";
import { reattachOrder } from "./reattach.ts";
import { answerRequest, type Opened, type OpenSpec, type RemoteTarget } from "./remote.ts";
import { TerminalView } from "./view.ts";

/** The page's prompts. Created with the module so a component can show them at once. */
export const prompts = new PromptStore();

/** The page's terminal, ready to be mounted. */
export interface TerminalApp {
  readonly view: TerminalView;
  /**
   * Gives the view a session: one that is already running if the page was reloaded,
   * otherwise a new shell. Call it once the view is mounted, since a new shell is
   * started at the view's size. Later calls do nothing.
   */
  connect(): void;
}

/** A session cannot be attached to for a reason that leaves the others worth trying. */
const TRY_ANOTHER = new Set(["already_attached", "not_found", "session_exited"]);

/** Where the page keeps the id of the session it shows, to find it again after a reload. */
const SHOWN_KEY = "lume.shown-session";

/** The parts of the keyboard-layout interface some webviews offer. */
interface KeyboardLike {
  getLayoutMap(): Promise<Iterable<[string, string]>>;
}

/** The keyboard layout as the webview reports it, where it can. */
async function keyboardLayout(): Promise<KeyLayout> {
  const keyboard = (navigator as Navigator & { keyboard?: KeyboardLike }).keyboard;
  try {
    if (keyboard) return new KeyLayout(new Map(await keyboard.getLayoutMap()));
  } catch {
    // Not allowed or not supported: the layout is learnt from what is typed instead.
  }
  return new KeyLayout();
}

function remembered(): string | null {
  try {
    return window.sessionStorage.getItem(SHOWN_KEY);
  } catch {
    // Storage may be refused. Then a reload shows whichever session it finds first.
    return null;
  }
}

function remember(session: string): void {
  try {
    window.sessionStorage.setItem(SHOWN_KEY, session);
  } catch {
    // As above.
  }
}

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** The page's sessions: the transport, and which session the view shows. */
class Sessions implements RemoteTarget {
  readonly #view: TerminalView;
  readonly #carrier: TauriCarrier;
  #client: TransportClient | undefined;
  /** Settles when the first session has been shown, or could not be. */
  #started: Promise<void> | undefined;

  constructor(view: TerminalView, carrier: TauriCarrier) {
    this.#view = view;
    this.#carrier = carrier;
  }

  /** Opens the transport and gives the view a session. Does so once. */
  connect(): void {
    this.#started ??= this.#connect().catch((error: unknown) => {
      this.#view.showMessage(`A session could not be started: ${reason(error)}`);
    });
  }

  async #connect(): Promise<void> {
    const client = await TransportClient.open(this.#carrier, {
      onFailure: (error) =>
        this.#view.showMessage(`The session can no longer be reached: ${error.message}`),
      onControl: (message) => this.#asked(message),
    });
    this.#client = client;
    // A session is started at the size that fits the view, so the view is laid out first.
    await this.#view.whenMeasured();
    // A page that was reloaded finds its sessions still running. Whether one is free is
    // found out by trying: the list may be a moment behind.
    for (const session of reattachOrder(await client.list(), remembered())) {
      try {
        await this.#view.attach(client, session.backend, session.id);
        remember(session.id);
        return;
      } catch (error) {
        if (!(error instanceof CommandError) || !TRY_ANOTHER.has(error.code)) throw error;
      }
    }
    await this.open({});
  }

  /** Carries out a request from the core and sends back the answer. */
  #asked(message: unknown): void {
    // A request that arrives while the page is still starting is about the session the
    // page is about to show, so it waits for that.
    const started = this.#started ?? Promise.resolve();
    void started.then(async () => {
      const answer = await answerRequest(this, message);
      if (!answer) return;
      this.#client?.answer(answer.id, answer.result).catch((error: unknown) => {
        console.error("an answer to the core was not delivered", error);
      });
    });
  }

  /** Starts a session and shows it in place of the one shown before, which carries on. */
  async open(spec: OpenSpec): Promise<Opened> {
    const client = this.#client;
    if (!client) throw new CommandError("no_window", "the page has not connected yet");
    await this.#view.whenMeasured();
    const previous = this.#view.session ?? null;
    const backend = client.defaultBackend;
    const session = await client.spawn(backend, {
      size: spec.size ?? this.#view.gridSize(),
      ...(spec.launcher ? { launcher: spec.launcher } : {}),
      ...(spec.cwd === undefined ? {} : { cwd: spec.cwd }),
      ...(spec.env ? { env: spec.env } : {}),
    });
    await this.#view.attach(client, backend, session, spec.size);
    remember(session);
    return { session, previous };
  }

  /** Starts a new shell where a program has ended. */
  restart(): void {
    this.open({}).catch((error: unknown) => {
      this.#view.showMessage(`A session could not be started: ${reason(error)}`);
    });
  }

  screen(session: string | undefined) {
    return this.#view.screen(session);
  }

  waitText(session: string | undefined, text: string, timeoutMs: number) {
    return this.#view.waitText(session, text, timeoutMs);
  }

  resizeTo(session: string | undefined, size: Parameters<TerminalView["resizeTo"]>[1]) {
    return this.#view.resizeTo(session, size);
  }
}

async function start(): Promise<TerminalApp> {
  checkFrameContract();
  const carrier = new TauriCarrier();
  const [vt, host, layout] = await Promise.all([
    Vt.instantiate(fetch(wasmUrl)),
    RenderHost.create(),
    keyboardLayout(),
  ]);
  // The view asks for a restart through the sessions, which are made from the view.
  const view: TerminalView = new TerminalView({
    vt,
    host,
    keys: new KeyEncoder(vt),
    platform: currentPlatform(),
    layout,
    appearance: DEFAULT_APPEARANCE,
    prompts,
    clipboard: {
      read: () => carrier.command<string | null>("clipboard_read"),
      write: (text) => carrier.command<void>("clipboard_write", { text }),
    },
    openLink: (url) => carrier.command<void>("open_link", { url }),
    developerTools: import.meta.env.DEV,
    restart: () => sessions.restart(),
  });
  const sessions = new Sessions(view, carrier);

  return { view, connect: () => sessions.connect() };
}

let app: Promise<TerminalApp> | undefined;

/** The page's terminal. The first call starts it; every call gets the same one. */
export function terminalApp(): Promise<TerminalApp> {
  app ??= start();
  return app;
}
