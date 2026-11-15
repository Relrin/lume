/**
 * Starting the terminal: once for each page, however often the component mounts.
 *
 * What is slow or can be had only once is done here: loading the terminal engine, asking
 * for a graphics device, opening the carrier. In development React mounts components
 * twice, and a second carrier would retire the first, so none of this may follow the
 * component's life.
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

async function connect(view: TerminalView, carrier: TauriCarrier): Promise<void> {
  const client = await TransportClient.open(carrier, {
    onFailure: (error) =>
      view.showMessage(`The session can no longer be reached: ${error.message}`),
  });
  // A page that was reloaded finds its sessions still running. Whether one is free is
  // found out by trying: the list may be a moment behind.
  for (const session of await client.list()) {
    if (session.exit !== null) continue;
    try {
      await view.attach(client, session.backend, session.id);
      return;
    } catch (error) {
      if (!(error instanceof CommandError) || !TRY_ANOTHER.has(error.code)) throw error;
    }
  }
  const session = await client.spawn(client.defaultBackend, { size: view.gridSize() });
  await view.attach(client, client.defaultBackend, session);
}

async function start(): Promise<TerminalApp> {
  checkFrameContract();
  const carrier = new TauriCarrier();
  const [vt, host, layout] = await Promise.all([
    Vt.instantiate(fetch(wasmUrl)),
    RenderHost.create(),
    keyboardLayout(),
  ]);
  const view = new TerminalView({
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
  });

  let connected = false;
  return {
    view,
    connect() {
      if (connected) return;
      connected = true;
      connect(view, carrier).catch((error: unknown) => {
        const reason = error instanceof Error ? error.message : String(error);
        view.showMessage(`A session could not be started: ${reason}`);
      });
    },
  };
}

let app: Promise<TerminalApp> | undefined;

/** The page's terminal. The first call starts it; every call gets the same one. */
export function terminalApp(): Promise<TerminalApp> {
  app ??= start();
  return app;
}
