/**
 * A stand-in for the core, inside the page.
 *
 * The window's page talks to the core through one carrier: commands one way, frames the
 * other. This is a carrier with nothing behind it but a layout and a configuration kept
 * here. A test tells it what output to send and what to change, and the page under test
 * cannot tell it from the application.
 *
 * Left alone it is one window with two tabs: two panes side by side, and one pane. A
 * test that needs another window to look at says how many panes each tab has, where the
 * tabs are listed, whether a pane floats, and so on.
 */

import { DEFAULT_TOKENS, LIGHT_TOKENS } from "@lume/tokens";

// The schema of the settings, as the core hands it to the settings window.
import schemaText from "../../../../schemas/config.schema.json?raw";
import type {
  CommandSpec,
  ConfigView,
  KeyBinding,
  Machine,
  Notice,
  Scheme,
  ProfileList,
  ShellEntry,
  ThemeEntry,
  TitleBar,
} from "../core/types.ts";
import type { ExitStatus, Launcher, SessionEvent } from "../transport/types.ts";
import type {
  Area,
  DockArea,
  Floating,
  Intent,
  LayoutChange,
  Node,
  PaneId,
  PaneNode,
  SplitId,
  Tab,
  TabId,
  Workspace,
} from "../layout/types.ts";
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

/** The whole a split's shares add up to, and an even half of it. */
const WHOLE = 1_000_000;
const HALF = WHOLE / 2;

/** How wide a dock area is, as the layout has it before anyone drags its edge. */
const DOCK_SIZE = 240;

/** The id of the panel that lists the tabs, and of the panel a test docks at a side. */
const TAB_LIST_PANEL = "lume.tabs";
const FIXTURE_PANEL = "fixture.panel";

/** Where the floating pane of a fixture floats, in the layout's units. */
const FLOAT_RECT = { x: 300_000, y: 250_000, w: 400_000, h: 400_000 };

/** How many panes a tab of a fixture may have. */
const TAB_PANES_MAX = 4;

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

/** The session of the pane with a number: sixteen hexadecimal digits. */
function sessionOfNumber(number: number): string {
  return number.toString(16).padStart(16, "0");
}

/** What a pane of a fixture runs, as the core would know it. */
export interface FixturePane {
  /** What its session was started to run. */
  readonly launcher?: Launcher;
  /** The profile it was started from. */
  readonly profile?: string;
  /** The directory it was started in. */
  readonly cwd?: string;
  /** Whether the pane says what it runs itself, as a pane started from a profile does. */
  readonly saved?: boolean;
}

/** The machine a fixture's page runs on. */
export const FIXTURE_MACHINE: Machine = { name: "FIXTURE", home: "C:\\Users\\fixture" };

/** A terminal pane that shows the session called `session`. */
function terminal(id: PaneId, session: string, setup: FixturePane = {}): PaneNode {
  const saved = setup.saved === true;
  return {
    type: "pane" as const,
    id,
    title: null,
    content: {
      type: "terminal" as const,
      backend: "local",
      session,
      profile: saved ? (setup.profile ?? null) : null,
      launcher: saved ? (setup.launcher ?? null) : null,
      cwd: setup.cwd ?? null,
      env: [],
    },
  };
}

/** An exit status as a frame carries it: how it ended, and the code or signal. */
function exitPayload(status: ExitStatus): Uint8Array {
  const bytes = new Uint8Array(5);
  const view = new DataView(bytes.buffer);
  if (status === "unknown") {
    view.setUint8(0, 2);
  } else if ("code" in status) {
    view.setUint8(0, 0);
    view.setUint32(1, status.code >>> 0, true);
  } else {
    view.setUint8(0, 1);
    view.setUint32(1, status.signal, true);
  }
  return bytes;
}

/** The sessions of the fixture's panes, when nothing is asked for. */
export const FIXTURE_SESSIONS: Readonly<Record<PaneId, string>> = {
  p1: sessionOfNumber(1),
  p2: sessionOfNumber(2),
  p3: sessionOfNumber(3),
};

/** The fixture's tabs: the first holds two panes side by side, the second one pane. */
export const FIXTURE_TABS = { first: "t1", second: "t2" } as const;

/** What a test may ask of the window it is given. Everything may be left out. */
export interface FakeCoreOptions {
  /**
   * How many panes each tab has, from one to four: alone, two side by side, one beside
   * two stacked, or four in a square. Left out, two tabs of two panes and one.
   */
  readonly tabs?: readonly number[];
  /** Where the tabs are listed. Left out, in the bar at the top. */
  readonly tabPlacement?: "top" | "left" | "right";
  /** Whether one more pane floats over the tabs. */
  readonly floating?: boolean;
  /** Whether one more pane is put away at the window's corner. */
  readonly minimised?: boolean;
  /** The side a panel of some plugin's is docked at. */
  readonly dockPanel?: Exclude<Area, "top">;
  /** What terminals are drawn with. Left out, the best the webview offers. */
  readonly tier?: "auto" | "webgpu" | "webgl2" | "canvas2d";
  /** Whether the look is the light or the dark one. Left out, dark. */
  readonly scheme?: Scheme;
  /** Who draws the title bar. Left out, the system, so the page draws no buttons. */
  readonly titleBar?: TitleBar;
  /** How many lines a terminal keeps once they have scrolled away. */
  readonly scrollbackLines?: number;
  /** The commands there are. Left out, none. */
  readonly commands?: readonly CommandSpec[];
  /** The keys in effect. Left out, none. */
  readonly bindings?: readonly KeyBinding[];
  /** What some of the panes run, by pane. Left out, a pane says nothing of it. */
  readonly panes?: Readonly<Record<PaneId, FixturePane>>;
  /** The profiles there are. Left out, a shell, a distribution and a host. */
  readonly profiles?: ProfileList;
  /** Settings of the page's own to lay over the fixture's, by section. */
  readonly settings?: Readonly<Record<string, Readonly<Record<string, unknown>>>>;
}

/** One tab of a fixture: its panes, in order, and the splits that divide it. */
interface TabShape {
  readonly id: TabId;
  readonly panes: readonly PaneId[];
  /** Builds the tab's tree, asking for each split's shares by its id. */
  readonly root: (shares: (split: SplitId) => readonly number[]) => Node;
}

/** The window a test asked for, worked out once. */
interface Arrangement {
  readonly tabs: readonly TabShape[];
  /** The pane that floats, and the one that is put away, if there are such. */
  readonly floating: PaneId | null;
  readonly minimised: PaneId | null;
  /** Every pane's session. */
  readonly sessions: ReadonlyMap<PaneId, string>;
  /** The number the layout would give the next thing it made. */
  readonly nextId: number;
}

function arrange(options: FakeCoreOptions): Arrangement {
  let panes = 0;
  let splits = 0;
  const sessions = new Map<PaneId, string>();
  const pane = (): PaneNode => {
    panes += 1;
    const id = `p${panes}`;
    sessions.set(id, sessionOfNumber(panes));
    return terminal(id, sessionOfNumber(panes), options.panes?.[id]);
  };
  const split = (): SplitId => {
    splits += 1;
    return `s${splits}`;
  };
  const divided = (
    id: SplitId,
    axis: "row" | "column",
    children: (shares: (split: SplitId) => readonly number[]) => readonly Node[],
  ): TabShape["root"] => {
    return (shares) => ({
      type: "split",
      id,
      axis,
      ratios: shares(id),
      children: children(shares),
    });
  };

  const tabs = (options.tabs ?? [2, 1]).map((count, index): TabShape => {
    if (!Number.isInteger(count) || count < 1 || count > TAB_PANES_MAX) {
      throw new Error(`a tab of a fixture has one to ${TAB_PANES_MAX} panes, not ${count}`);
    }
    const id = `t${index + 1}`;
    const first = panes;
    let root: TabShape["root"];
    if (count === 1) {
      const only = pane();
      root = () => only;
    } else if (count === 2) {
      const outer = split();
      const [left, right] = [pane(), pane()];
      root = divided(outer, "row", () => [left, right]);
    } else if (count === 3) {
      const outer = split();
      const left = pane();
      const inner = split();
      const [top, bottom] = [pane(), pane()];
      const stacked = divided(inner, "column", () => [top, bottom]);
      root = divided(outer, "row", (shares) => [left, stacked(shares)]);
    } else {
      const outer = split();
      const leftSplit = split();
      const leftPanes = [pane(), pane()];
      const rightSplit = split();
      const rightPanes = [pane(), pane()];
      const left = divided(leftSplit, "column", () => leftPanes);
      const right = divided(rightSplit, "column", () => rightPanes);
      root = divided(outer, "row", (shares) => [left(shares), right(shares)]);
    }
    const mine: PaneId[] = [];
    for (let number = first + 1; number <= panes; number++) mine.push(`p${number}`);
    return { id, panes: mine, root };
  });

  const extra = (): PaneId => pane().id;
  const floating = options.floating === true ? extra() : null;
  const minimised = options.minimised === true ? extra() : null;
  return {
    tabs,
    floating,
    minimised,
    sessions,
    // The fixture nothing was asked of has always said ten.
    nextId: Math.max(10, panes + splits + tabs.length + 2),
  };
}

/** The themes a fixture's settings can choose from. */
const THEMES: readonly ThemeEntry[] = [
  { id: "lume-dark", name: "Lume Dark", scheme: "dark", builtin: true },
  { id: "lume-light", name: "Lume Light", scheme: "light", builtin: true },
];

/** The shells a fixture's machine has. */
const SHELLS: readonly ShellEntry[] = [
  { name: "PowerShell", program: "C:\\Program Files\\PowerShell\\7\\pwsh.exe" },
  { name: "Command Prompt", program: "C:\\Windows\\System32\\cmd.exe" },
];

/** The ways a fixture's machine has to start a terminal. */
const PROFILES: ProfileList = {
  revision: 1,
  default: null,
  notes: [],
  looked: true,
  profiles: [
    {
      id: "auto.pwsh",
      name: "PowerShell",
      launcher: {
        type: "shell",
        program: "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
        args: [],
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
      id: "auto.ssh.dev-staging",
      name: "dev-staging",
      launcher: { type: "ssh", host: "dev-staging" },
      hidden: false,
      origin: "generated",
      detail: "from ~/.ssh/config",
    },
  ],
};

/** The fixed-width fonts a fixture's machine has. */
const FONTS: readonly string[] = ["Cascadia Mono", "Consolas", "JetBrains Mono"];

/** A core that is only a layout, a configuration and whatever a test sends through it. */
export class FakeCore implements Carrier {
  readonly #options: FakeCoreOptions;
  readonly #fixture: Arrangement;
  /** The tab on show, and the pane that has the keyboard. */
  #active: TabId;
  #focus: PaneId;
  /** The shares of the splits a test has changed, and the sizes of the dock areas. */
  readonly #shares = new Map<SplitId, readonly number[]>();
  readonly #dockSizes = new Map<Area, number>();
  #scheme: Scheme;
  #configRevision = 1;
  /** The stream the page attached to each session on. */
  readonly #streams = new Map<string, number>();
  #receive: (frames: Uint8Array) => void = () => undefined;
  #revision = 1;
  #profiles: ProfileList;
  /** The panes whose program has ended. */
  readonly #ended = new Set<PaneId>();
  /** Commands the page called that this core has no answer for. */
  readonly unanswered: string[] = [];

  constructor(options: FakeCoreOptions = {}) {
    this.#options = options;
    this.#fixture = arrange(options);
    const first = this.#fixture.tabs[0];
    this.#active = first?.id ?? FIXTURE_TABS.first;
    this.#focus = first?.panes[0] ?? "p1";
    this.#scheme = options.scheme ?? "dark";
    this.#profiles = options.profiles ?? PROFILES;
  }

  /** The pane that shows a session of the fixture. */
  #paneOf(session: string): PaneId | undefined {
    for (const [pane, shown] of this.#fixture.sessions) if (shown === session) return pane;
    return undefined;
  }

  /** Every pane of the fixture, those of the tabs first. */
  get panes(): readonly PaneId[] {
    return [...this.#fixture.sessions.keys()];
  }

  /** The panes of one tab, in order. */
  panesOf(tab: TabId): readonly PaneId[] {
    return this.#fixture.tabs.find((found) => found.id === tab)?.panes ?? [];
  }

  /** The tabs of the fixture, in order. */
  get tabs(): readonly TabId[] {
    return this.#fixture.tabs.map((tab) => tab.id);
  }

  /** The session a pane of the fixture shows. */
  sessionOf(pane: PaneId): string | undefined {
    return this.#fixture.sessions.get(pane);
  }

  #dock(area: Area): DockArea {
    const placement = this.#options.tabPlacement ?? "top";
    const panels = [];
    // Along the top the tabs are in the bar itself; at a side they are a panel.
    if (placement === area && area !== "top") {
      panels.push({ id: TAB_LIST_PANEL, content: { type: "tab_list" as const } });
    }
    if (this.#options.dockPanel === area) {
      panels.push({
        id: FIXTURE_PANEL,
        content: { type: "plugin_view" as const, plugin: "fixture", view: "panel" },
      });
    }
    return { panels, size: this.#dockSizes.get(area) ?? DOCK_SIZE, collapsed: false };
  }

  /** The workspace as it is now. */
  #workspace(): Workspace {
    const shares = (split: SplitId): readonly number[] => this.#shares.get(split) ?? [HALF, HALF];
    const tabs: Tab[] = this.#fixture.tabs.map((tab) => ({
      id: tab.id,
      title: null,
      root: tab.root(shares),
      zoomed: null,
      last_focus: tab.panes[0] ?? null,
    }));
    const floats: Floating[] = [];
    const float = (pane: PaneId | null, minimised: Floating["minimised"]): void => {
      if (pane === null) return;
      floats.push({
        pane: terminal(pane, this.#fixture.sessions.get(pane) ?? "", this.#options.panes?.[pane]),
        rect: FLOAT_RECT,
        minimised,
      });
    };
    float(this.#fixture.minimised, "tab");
    float(this.#fixture.floating, null);
    return {
      id: "00000000000000aa",
      name: "fixture",
      windows: [
        {
          id: WINDOW,
          bounds: null,
          dock: {
            top: this.#dock("top"),
            left: this.#dock("left"),
            right: this.#dock("right"),
            bottom: this.#dock("bottom"),
          },
          centre: { tabs, active: this.#active },
          floating: floats,
          focus: this.#focus,
          history: [this.#focus],
        },
      ],
      focused_window: WINDOW,
      scratch: null,
      next_id: this.#fixture.nextId,
    };
  }

  /** The built-in look, with a cursor that does not blink: nothing moves unless told to. */
  #config(): ConfigView {
    const light = this.#scheme === "light";
    return {
      revision: this.#configRevision,
      platform: "windows",
      scheme: this.#scheme,
      theme: { id: light ? "lume-light" : "lume-dark", scheme: this.#scheme },
      config: {
        appearance: { "pane-header": "always", ...this.#options.settings?.["appearance"] },
        terminal: {
          "scrollback-lines": this.#options.scrollbackLines ?? 1000,
          "cursor-style": "block",
          "cursor-blink": false,
          ...this.#options.settings?.["terminal"],
        },
        layout: { "tab-placement": this.#options.tabPlacement ?? "top" },
        renderer: { tier: this.#options.tier ?? "auto" },
      },
      sources: {},
      problems: [],
      tokens: light ? LIGHT_TOKENS : DEFAULT_TOKENS,
      keymap: { bindings: this.#options.bindings ?? [] },
      // Unless a test says otherwise the system draws the title bar, so the page draws
      // no window buttons of its own.
      title_bar: this.#options.titleBar ?? "system",
    };
  }

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
          workspace: this.#workspace(),
          machine: FIXTURE_MACHINE,
        });
      case "panes_busy": {
        const asked = Array.isArray(args["panes"]) ? (args["panes"] as PaneId[]) : [];
        return answer(
          asked.filter((pane) => this.#fixture.sessions.has(pane) && !this.#ended.has(pane)),
        );
      }
      case "config_view":
        return answer(this.#config());
      case "commands_list":
        return answer(this.#options.commands ?? []);
      case "window_state":
        return answer({ maximised: false, fullscreen: false, focused: true });
      case "workspaces_list":
        return answer({ active: "fixture", workspaces: [{ name: "fixture", root: null }] });
      case "session_attach": {
        const session = String(args["session"]);
        this.#streams.set(session, Number(args["stream"]));
        const pane = this.#paneOf(session);
        const setup = pane === undefined ? undefined : this.#options.panes?.[pane];
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
          ...(setup?.launcher ? { launcher: setup.launcher } : {}),
          ...(setup?.profile !== undefined ? { profile: setup.profile } : {}),
        });
      }
      case "layout_apply":
        this.#apply(args["intent"] as Intent);
        return answer({
          epoch: 1,
          revision: this.#revision,
          created: { pane: null, tab: null, window: null },
        });
      case "config_schema":
        return answer(JSON.parse(schemaText));
      case "themes_list":
        return answer(THEMES);
      case "shells_list":
        return answer(SHELLS);
      case "profiles_list":
        return answer(this.#profiles);
      case "font_families":
        return answer(FONTS);
      // No picture is behind the panes: no bytes.
      case "background_image":
        return answer(new ArrayBuffer(0));
      case "command_run":
      case "page_reply":
      case "clipboard_write":
      case "window_drag":
      case "window_control":
      case "config_set":
      case "config_unset":
      case "config_open":
        return answer(null);
      default:
        this.unanswered.push(name);
        return Promise.reject({ code: "unsupported", message: `the fake core has no ${name}` });
    }
  }

  /**
   * Does the little of an intent a test's window needs done: a split's shares, which
   * pane has the keyboard, and how wide a dock area is. The page is told as the core
   * tells it. Anything else is taken and changes nothing.
   */
  #apply(intent: Intent | undefined): void {
    if (!intent) return;
    switch (intent.type) {
      case "resize":
        this.#shares.set(intent.split, intent.ratios);
        this.#changed([{ type: "ratios", split: intent.split, ratios: intent.ratios }]);
        break;
      case "focus":
        if (intent.target.type !== "pane" || intent.target.pane === this.#focus) break;
        if (!this.#fixture.sessions.has(intent.target.pane)) break;
        this.#focus = intent.target.pane;
        this.#changed([{ type: "focus", window: WINDOW, pane: this.#focus }]);
        break;
      case "panel_resize":
        this.#dockSizes.set(intent.area, intent.size);
        this.#changed([
          { type: "dock_size", window: WINDOW, area: intent.area, size: intent.size },
        ]);
        break;
      default:
        break;
    }
  }

  /** Whether the page has attached to the session of every pane. */
  attached(): boolean {
    return [...this.#fixture.sessions.values()].every((session) => this.#streams.has(session));
  }

  /**
   * Sends output to the page as the session of `pane`, in frames no larger than a
   * frame may be, as the core does. Returns how many bytes went.
   */
  output(pane: PaneId, bytes: Uint8Array): number {
    const stream = this.#streamOf(pane);
    if (stream === undefined) return 0;
    for (let at = 0; at < bytes.length; at += LIMITS.maxPayload) {
      this.#receive(frame(stream, Kind.Data, bytes.subarray(at, at + LIMITS.maxPayload)));
    }
    return bytes.length;
  }

  /** The stream the page attached to the session of `pane` on, once it has. */
  #streamOf(pane: PaneId): number | undefined {
    return this.#streams.get(this.#fixture.sessions.get(pane) ?? "");
  }

  /**
   * Sends something the session of `pane` reported, as the core passes it on. Returns
   * whether the page had attached to that session to be told.
   */
  event(pane: PaneId, event: SessionEvent): boolean {
    const stream = this.#streamOf(pane);
    if (stream === undefined) return false;
    this.#receive(frame(stream, Kind.Event, new TextEncoder().encode(JSON.stringify(event))));
    return true;
  }

  /** Ends the program of `pane`'s session, as the core says when it does. */
  exit(pane: PaneId, status: ExitStatus): boolean {
    const stream = this.#streamOf(pane);
    if (stream === undefined) return false;
    this.#ended.add(pane);
    this.#receive(frame(stream, Kind.Exit, exitPayload(status)));
    return true;
  }

  /** Changes the profiles there are, as the core does when it finds others. */
  profilesChanged(list: Omit<ProfileList, "revision">): void {
    this.#profiles = { ...list, revision: this.#profiles.revision + 1 };
    this.#notify({ type: "profiles_changed", ...this.#profiles });
  }

  #notify(notice: Notice): void {
    const json = new TextEncoder().encode(JSON.stringify({ notify: notice }));
    this.#receive(frame(CONTROL_STREAM, Kind.Control, json));
  }

  #changed(changes: readonly LayoutChange[]): void {
    this.#revision += 1;
    this.#notify({ type: "layout_changed", epoch: 1, revision: this.#revision, changes });
  }

  /** Shows a tab, as the core does: the tab's first pane takes the keyboard. */
  activate(tab: TabId): void {
    const focus = this.#fixture.tabs.find((found) => found.id === tab)?.panes[0];
    if (focus === undefined) return;
    this.#active = tab;
    this.#focus = focus;
    this.#changed([
      { type: "tabs", window: WINDOW, order: this.tabs, active: tab },
      { type: "focus", window: WINDOW, pane: focus },
    ]);
  }

  /** Changes between the light look and the dark one, as the system's scheme changing does. */
  setScheme(scheme: Scheme): void {
    if (scheme === this.#scheme) return;
    this.#scheme = scheme;
    this.#configRevision += 1;
    this.#notify({
      type: "config_changed",
      revision: this.#configRevision,
      changed: ["scheme", "theme", "tokens"],
      view: this.#config(),
    });
  }
}
