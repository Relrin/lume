/**
 * A stand-in for the core, inside the page.
 *
 * The window's page talks to the core through one carrier: commands one way, frames the
 * other. This is a carrier with nothing behind it but a layout and a configuration kept
 * here. A test tells it what output to send and what to change, and the page under test
 * cannot tell it from the application.
 */

import { DEFAULT_TOKENS } from "@lume/tokens";

import type { ConfigView, Notice } from "../core/types.ts";
import type { LayoutChange, PaneId, TabId, Workspace } from "../layout/types.ts";
import type { Carrier, CarrierOpened } from "../transport/carrier.ts";
import { CONTROL_STREAM, HEADER_LEN, Kind } from "../transport/frames.ts";

/** The protocol's sizes, as the core reports them. */
const LIMITS = {
  maxPayload: 64 * 1024,
  maxControlMessage: 1024 * 1024,
  maxHydrate: 64 * 1024 * 1024,
};
const CREDIT_WINDOW = 512 * 1024;

const WINDOW = "w1";

/** One frame: its header and its payload. */
function frame(stream: number, kind: number, payload: Uint8Array): Uint8Array {
  const bytes = new Uint8Array(HEADER_LEN + payload.length);
  const view = new DataView(bytes.buffer);
  view.setUint32(0, stream, true);
  view.setUint8(4, kind);
  view.setUint32(5, payload.length, true);
  bytes.set(payload, HEADER_LEN);
  return bytes;
}

/** A terminal pane that shows the session called `session`. */
function terminal(id: PaneId, session: string) {
  return {
    type: "pane" as const,
    id,
    title: null,
    content: {
      type: "terminal" as const,
      backend: "local",
      session,
      profile: null,
      launcher: null,
      cwd: null,
      env: [],
    },
  };
}

function dock() {
  return { panels: [], size: 240, collapsed: false };
}

/** The sessions of the fixture's panes. */
export const FIXTURE_SESSIONS: Readonly<Record<PaneId, string>> = {
  p1: "0000000000000001",
  p2: "0000000000000002",
  p3: "0000000000000003",
};

/** The fixture's tabs: the first holds two panes side by side, the second one pane. */
export const FIXTURE_TABS = { first: "t1", second: "t2" } as const;

/** The pane of each of the fixture's tabs that takes the keyboard when the tab is shown. */
const TAB_FOCUS: Readonly<Record<TabId, PaneId>> = { t1: "p1", t2: "p3" };

/** A workspace with one window, two tabs and three terminal panes, showing `active`. */
function fixtureWorkspace(active: TabId): Workspace {
  const focus = TAB_FOCUS[active] ?? "p1";
  const session = (pane: PaneId): string => FIXTURE_SESSIONS[pane] ?? "";
  return {
    id: "00000000000000aa",
    name: "fixture",
    windows: [
      {
        id: WINDOW,
        bounds: null,
        dock: { top: dock(), left: dock(), right: dock(), bottom: dock() },
        centre: {
          tabs: [
            {
              id: FIXTURE_TABS.first,
              title: null,
              root: {
                type: "split",
                id: "s1",
                axis: "row",
                ratios: [500_000, 500_000],
                children: [terminal("p1", session("p1")), terminal("p2", session("p2"))],
              },
              zoomed: null,
              last_focus: "p1",
            },
            {
              id: FIXTURE_TABS.second,
              title: null,
              root: terminal("p3", session("p3")),
              zoomed: null,
              last_focus: "p3",
            },
          ],
          active,
        },
        floating: [],
        focus,
        history: [focus],
      },
    ],
    focused_window: WINDOW,
    scratch: null,
    next_id: 10,
  };
}

/** The built-in look, with a cursor that does not blink: nothing moves unless told to. */
function fixtureConfig(): ConfigView {
  return {
    revision: 1,
    platform: "windows",
    scheme: "dark",
    theme: { id: "lume-dark", scheme: "dark" },
    config: {
      appearance: { "pane-header": "always" },
      terminal: { "scrollback-lines": 1000, "cursor-style": "block", "cursor-blink": false },
      layout: { "tab-placement": "top" },
      renderer: { tier: "auto" },
    },
    sources: {},
    problems: [],
    tokens: DEFAULT_TOKENS,
    keymap: { bindings: [] },
  };
}

/** A core that is only a layout, a configuration and whatever a test sends through it. */
export class FakeCore implements Carrier {
  /** The tab on show. */
  #active: TabId = FIXTURE_TABS.first;
  readonly #config = fixtureConfig();
  /** The stream the page attached to each session on. */
  readonly #streams = new Map<string, number>();
  #receive: (frames: Uint8Array) => void = () => undefined;
  #revision = 1;
  /** Commands the page called that this core has no answer for. */
  readonly unanswered: string[] = [];

  open(receive: (frames: Uint8Array) => void): Promise<CarrierOpened> {
    this.#receive = receive;
    return Promise.resolve({
      carrier: 1,
      limits: LIMITS,
      creditWindow: CREDIT_WINDOW,
      defaultBackend: "local",
    });
  }

  /** What the page sends on its streams is taken and dropped: nothing here reads it. */
  send(): Promise<void> {
    return Promise.resolve();
  }

  command<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
    const answer = (value: unknown) => Promise.resolve(value as T);
    switch (name) {
      case "layout_view":
        return answer({
          window: WINDOW,
          epoch: 1,
          revision: this.#revision,
          workspace: fixtureWorkspace(this.#active),
        });
      case "config_view":
        return answer(this.#config);
      case "commands_list":
        return answer([]);
      case "session_attach":
        this.#streams.set(String(args["session"]), Number(args["stream"]));
        return answer({
          size: { cols: 80, rows: 24 },
          resize_pulls_scrollback: false,
          hydrate: "none",
          caps: {
            persistent: false,
            multi_client: false,
            server_layout: false,
            responder: "client",
            hydrate: { snapshot: false, vt_replay: false },
          },
        });
      case "layout_apply":
      case "command_run":
      case "page_reply":
      case "clipboard_write":
        return answer(null);
      default:
        this.unanswered.push(name);
        return Promise.reject({ code: "unsupported", message: `the fake core has no ${name}` });
    }
  }

  /** Whether the page has attached to the session of every pane. */
  attached(): boolean {
    return Object.values(FIXTURE_SESSIONS).every((session) => this.#streams.has(session));
  }

  /** Sends output to the page as the session of `pane`. Returns how many bytes went. */
  output(pane: PaneId, bytes: Uint8Array): number {
    const stream = this.#streams.get(FIXTURE_SESSIONS[pane] ?? "");
    if (stream === undefined) return 0;
    this.#receive(frame(stream, Kind.Data, bytes));
    return bytes.length;
  }

  #notify(notice: Notice): void {
    const json = new TextEncoder().encode(JSON.stringify({ notify: notice }));
    this.#receive(frame(CONTROL_STREAM, Kind.Control, json));
  }

  /** Shows a tab, as the core does: the tab's last pane takes the keyboard. */
  activate(tab: TabId): void {
    const focus = TAB_FOCUS[tab];
    if (focus === undefined) return;
    this.#active = tab;
    const changes: LayoutChange[] = [
      {
        type: "tabs",
        window: WINDOW,
        order: [FIXTURE_TABS.first, FIXTURE_TABS.second],
        active: tab,
      },
      { type: "focus", window: WINDOW, pane: focus },
    ];
    this.#revision += 1;
    this.#notify({ type: "layout_changed", epoch: 1, revision: this.#revision, changes });
  }
}
