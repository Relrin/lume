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
  DetachedSession,
  KeyBinding,
  Machine,
  Notice,
  PanelRowData,
  Scheme,
  ProfileList,
  ShellEntry,
  ThemeEntry,
  TitleBar,
  UiContributions,
} from "../core/types.ts";
import type { ExitStatus, Launcher, SessionEvent } from "../transport/types.ts";
import type {
  Area,
  DockArea,
  Floating,
  Group,
  GroupId,
  Intent,
  LayoutChange,
  Node,
  PaneId,
  PaneNode,
  Shelved,
  SplitId,
  Tab,
  TabColor,
  TabId,
  Workspace,
} from "../layout/types.ts";
import type { Carrier, CarrierOpened } from "../transport/carrier.ts";
import { CONTROL_STREAM, HEADER_LEN, Kind } from "../transport/frames.ts";
import type { AddonEntry } from "../settings/addons.ts";

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

/** A group of a fixture's tabs: which tabs, by their places, and how it looks. */
export interface FixtureGroup extends Omit<Group, "id"> {
  /** The places of its tabs among the fixture's tabs, counted from nought. */
  readonly tabs: readonly number[];
}

/** How one of a fixture's tabs looks: its name, its colour, whether it is locked or muted. */
export interface FixtureTabLook {
  readonly title?: string;
  readonly color?: TabColor;
  readonly locked?: boolean;
  readonly muted?: boolean;
}

/** What a test may ask of the window it is given. Everything may be left out. */
export interface FakeCoreOptions {
  /** The groups the tabs are in. Left out, none. */
  readonly groups?: readonly FixtureGroup[];
  /** How some of the tabs look, by tab. */
  readonly tabLooks?: Readonly<Record<TabId, FixtureTabLook>>;
  /** The kept groups that were closed. Left out, none. */
  readonly shelf?: readonly Shelved[];
  /** The tab on show. Left out, the first. */
  readonly activeTab?: TabId;
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
  /** The sessions no pane shows. Left out, none. */
  readonly detached?: readonly DetachedSession[];
  /** Settings of the page's own to lay over the fixture's, by section. */
  readonly settings?: Readonly<Record<string, Readonly<Record<string, unknown>>>>;
  /** What plugins add to the window. Left out, nothing. */
  readonly contributions?: UiContributions;
  /** The plugins' pictures and shaders, by the hashes the contributions name. */
  readonly assets?: Readonly<Record<string, string>>;
  /**
   * The rows a plugin gives when a row of its panel is opened, by panel and then by
   * the ids down to the row, joined with `/`. A row not named here is never answered.
   */
  readonly lazyRows?: Readonly<Record<string, Readonly<Record<string, readonly PanelRowData[]>>>>;
  /** A plugin's panel docked at a side, besides the fixture's own. */
  readonly pluginPanel?: {
    readonly id: string;
    readonly plugin: string;
    readonly view: string;
    readonly area: Exclude<Area, "top">;
  };
  /** Whether the stand-in refuses to open plugins' views, as for a plugin gone. */
  readonly refuseViews?: boolean;
  /** The settings' schema, as plugins make it. Left out, the application's own. */
  readonly schema?: unknown;
  /** The plugins installed, as the settings window lists them. Left out, none. */
  readonly addons?: readonly AddonEntry[];
}

/** Rows with the rows beneath the row at `path` put in, as a plugin gives them. */
function withChildren(
  rows: readonly PanelRowData[],
  path: readonly string[],
  children: readonly PanelRowData[],
): readonly PanelRowData[] {
  const [first, ...rest] = path;
  return rows.map((row) => {
    if (row.id !== first) return row;
    return rest.length === 0
      ? { ...row, children, loaded: true }
      : { ...row, children: withChildren(row.children ?? [], rest, children) };
  });
}

/** What a window is told plugins add when none is installed. */
export const NO_CONTRIBUTIONS: UiContributions = {
  revision: 0,
  plugins: [],
  menus: {},
  status: [],
  panels: [],
  pane_views: [],
  openers: [],
  images: {},
  motions: [],
  cursor_effects: [],
  values: [],
  rows: {},
  contexts: {},
};

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

/** What the fake core keeps of a tab besides its tree. */
interface TabFacts {
  title: string | null;
  color: TabColor | null;
  locked: boolean;
  muted: boolean;
  group: GroupId | null;
}

/** The number the fixture's groups are counted from: past whatever else it numbers. */
const GROUP_IDS_FROM = 101;

/** What the layout of the fake core's tabs is, to compare one moment with another. */
interface TabsNow {
  readonly order: readonly TabId[];
  readonly active: TabId;
  readonly facts: ReadonlyMap<TabId, TabFacts>;
  readonly groups: readonly Group[];
}

/** A core that is only a layout, a configuration and whatever a test sends through it. */
export class FakeCore implements Carrier {
  readonly #options: FakeCoreOptions;
  /** The rows of panels a row of which was opened, as they are since. */
  readonly #panelRows = new Map<string, readonly PanelRowData[]>();
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
  /** The sessions no pane shows, less those a command has attached to or ended. */
  #detached: DetachedSession[];
  /** The panes whose program has ended. */
  readonly #ended = new Set<PaneId>();
  /** Commands the page called that this core has no answer for. */
  readonly unanswered: string[] = [];
  /** The order of the tabs, what each is besides its tree, and the groups. */
  #order: TabId[];
  readonly #facts = new Map<TabId, TabFacts>();
  #groups: Group[] = [];
  #nextGroup = GROUP_IDS_FROM;
  /** The commands the page ran, with their arguments, in order. */
  readonly ran: { readonly id: string; readonly args: unknown }[] = [];
  /** What the page last said it has on screen of the plugins' things. */
  visible: readonly string[] = [];

  constructor(options: FakeCoreOptions = {}) {
    this.#options = options;
    this.#fixture = arrange(options);
    const first = this.#fixture.tabs[0];
    this.#active = first?.id ?? FIXTURE_TABS.first;
    this.#focus = first?.panes[0] ?? "p1";
    this.#scheme = options.scheme ?? "dark";
    this.#profiles = options.profiles ?? PROFILES;
    this.#detached = [...(options.detached ?? [])];
    this.#order = this.#fixture.tabs.map((tab) => tab.id);
    for (const tab of this.#order) {
      const look = options.tabLooks?.[tab];
      this.#facts.set(tab, {
        title: look?.title ?? null,
        color: look?.color ?? null,
        locked: look?.locked ?? false,
        muted: look?.muted ?? false,
        group: null,
      });
    }
    for (const { tabs, ...look } of options.groups ?? []) {
      const id = `g${this.#nextGroup++}`;
      this.#groups.push({ ...look, id });
      for (const at of tabs) {
        const facts = this.#facts.get(this.#order[at] ?? "");
        if (facts) facts.group = id;
      }
    }
    if (options.activeTab !== undefined) {
      const shown = this.#fixture.tabs.find((tab) => tab.id === options.activeTab);
      if (shown) {
        this.#active = shown.id;
        this.#focus = shown.panes[0] ?? this.#focus;
      }
    }
    this.#tidy();
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
    return this.#order;
  }

  /** The groups of the fixture's tabs, in order. */
  get groups(): readonly Group[] {
    return this.#groups;
  }

  /** The session a pane of the fixture shows. */
  sessionOf(pane: PaneId): string | undefined {
    return this.#fixture.sessions.get(pane);
  }

  #dock(area: Area): DockArea {
    const placement = this.#options.tabPlacement ?? "top";
    const panels = [];
    // The top bar's three places for status items, as the core gives every window.
    if (area === "top") {
      for (const slot of ["topbar.left", "topbar.center", "topbar.right"] as const) {
        panels.push({ id: `lume.${slot}`, content: { type: "slot" as const, slot } });
      }
    }
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
    const plugin = this.#options.pluginPanel;
    if (plugin?.area === area) {
      panels.push({
        id: plugin.id,
        content: { type: "plugin_view" as const, plugin: plugin.plugin, view: plugin.view },
      });
    }
    return { panels, size: this.#dockSizes.get(area) ?? DOCK_SIZE, collapsed: false };
  }

  /** The workspace as it is now. */
  #workspace(): Workspace {
    const shares = (split: SplitId): readonly number[] => this.#shares.get(split) ?? [HALF, HALF];
    const tabs: Tab[] = this.#order.flatMap((id): Tab[] => {
      const tab = this.#fixture.tabs.find((found) => found.id === id);
      if (!tab) return [];
      const facts = this.#facts.get(id);
      return [
        {
          id: tab.id,
          title: facts?.title ?? null,
          root: tab.root(shares),
          zoomed: null,
          last_focus: tab.panes[0] ?? null,
          ...(facts?.color ? { color: facts.color } : {}),
          ...(facts?.locked ? { locked: true } : {}),
          ...(facts?.muted ? { muted: true } : {}),
          ...(facts?.group ? { group: facts.group } : {}),
        },
      ];
    });
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
          centre: {
            tabs,
            active: this.#active,
            ...(this.#groups.length > 0 ? { groups: this.#groups } : {}),
          },
          floating: floats,
          focus: this.#focus,
          history: [this.#focus],
        },
      ],
      focused_window: WINDOW,
      scratch: null,
      next_id: this.#fixture.nextId,
      ...(this.#options.shelf && this.#options.shelf.length > 0
        ? { shelf: this.#options.shelf }
        : {}),
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
        plugins: { ...this.#options.settings?.["plugins"] },
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
          host: { connected: true },
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
      case "about_info":
        return answer({
          appVersion: "0.0.0",
          os: "windows",
          arch: "x86_64",
          engine: "WebView2",
          engineVersion: "140.0.0.0",
          proto: 2,
          host: { version: "0.0.0", proto: 2, daemon: true, connected: true },
          rexVersion: null,
        });
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
        return answer(this.#options.schema ?? JSON.parse(schemaText));
      case "ui_contributions":
        return answer(this.#options.contributions ?? NO_CONTRIBUTIONS);
      case "ui_visible":
        this.visible = Array.isArray(args["items"]) ? (args["items"] as string[]) : [];
        return answer(null);
      case "view_opened":
        this.ran.push({ id: name, args });
        if (this.#options.refuseViews) {
          return Promise.reject(new Error("the plugin's view is not there any more"));
        }
        return answer(`w-1/${String(args["view"])}`);
      case "view_closed":
        return answer(null);
      case "view_state":
        this.ran.push({ id: name, args });
        return answer(null);
      case "view_stream_open":
        return Promise.reject(new Error("there is no such grant"));
      case "view_call":
        this.ran.push({ id: name, args });
        return answer(null);
      case "ui_assets": {
        const held = this.#options.assets ?? {};
        const hashes = Array.isArray(args["hashes"]) ? (args["hashes"] as string[]) : [];
        return answer(
          Object.fromEntries(
            hashes.filter((hash) => Object.hasOwn(held, hash)).map((hash) => [hash, held[hash]]),
          ),
        );
      }
      case "ui_panel_expand": {
        this.ran.push({ id: name, args });
        const panel = String(args["panel"]);
        const path = Array.isArray(args["path"]) ? (args["path"] as string[]) : [];
        const given = this.#options.lazyRows?.[panel]?.[path.join("/")];
        if (given) {
          this.#panelRows.set(
            panel,
            withChildren(
              this.#panelRows.get(panel) ?? this.#options.contributions?.rows[panel] ?? [],
              path,
              given,
            ),
          );
          // As the core does: the panel's rows whole, a moment later.
          queueMicrotask(() =>
            this.#notify({
              type: "plugin_frame",
              plugin: panel,
              statuses: [],
              panels: { [panel]: this.#panelRows.get(panel) ?? [] },
              contexts: {},
            }),
          );
        }
        return answer(null);
      }
      case "addons_list":
        return answer({ plugins: this.#options.addons ?? [] });
      case "addon_set_enabled":
      case "addon_remove":
      case "addon_move":
      case "addon_cancel":
        this.ran.push({ id: name, args });
        return answer(null);
      case "themes_list":
        return answer(THEMES);
      case "shells_list":
        return answer(SHELLS);
      case "profiles_list":
        return answer(this.#profiles);
      case "sessions_detached":
        return answer({ sessions: [...this.#detached] });
      case "font_families":
        return answer(FONTS);
      // No picture is behind the panes: no bytes.
      case "background_image":
        return answer(new ArrayBuffer(0));
      case "command_run":
        this.ran.push({ id: String(args["id"]), args: args["args"] ?? null });
        this.#command(String(args["id"]), args["args"]);
        return answer(null);
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
      case "tab_activate":
        if (intent.target.type === "id") this.activate(intent.target.tab);
        break;
      case "tab_place":
        this.#tabs(() => {
          const facts = this.#facts.get(intent.tab);
          if (!facts) return;
          this.#order = this.#order.filter((tab) => tab !== intent.tab);
          this.#order.splice(Math.min(intent.index, this.#order.length), 0, intent.tab);
          facts.group = intent.group ?? null;
        });
        break;
      case "group_move":
        this.#tabs(() => {
          const block = this.#order.filter((tab) => this.#facts.get(tab)?.group === intent.group);
          const rest = this.#order.filter((tab) => this.#facts.get(tab)?.group !== intent.group);
          rest.splice(Math.min(intent.index, rest.length), 0, ...block);
          this.#order = rest;
        });
        break;
      default:
        break;
    }
  }

  /** The tab a command acts on: the one it names, or the one on show. */
  #tabOf(args: unknown): TabId {
    const named = (args as { tab?: unknown } | null)?.tab;
    return typeof named === "string" ? named : this.#active;
  }

  /** The group a command acts on: the one it names, or the group of the tab on show. */
  #groupOf(args: unknown): GroupId | null {
    const named = (args as { group?: unknown } | null)?.group;
    return typeof named === "string" ? named : (this.#facts.get(this.#active)?.group ?? null);
  }

  /** Does the little of a command about tabs and groups the scenes need done. */
  #command(id: string, args: unknown): void {
    const given = (args ?? {}) as Record<string, unknown>;
    const tab = this.#tabOf(args);
    const facts = this.#facts.get(tab);
    const group = this.#groupOf(args);
    const editGroup = (edit: (found: Group) => Group) => {
      this.#groups = this.#groups.map((found) => (found.id === group ? edit(found) : found));
    };
    switch (id) {
      case "lume.tab.color":
        this.#tabs(() => {
          if (facts) facts.color = (given["color"] as TabColor | null) ?? null;
        });
        return;
      case "lume.tab.lock":
        this.#tabs(() => {
          if (facts) facts.locked = typeof given["on"] === "boolean" ? given["on"] : !facts.locked;
        });
        return;
      case "lume.tab.mute":
        this.#tabs(() => {
          if (facts) facts.muted = typeof given["on"] === "boolean" ? given["on"] : !facts.muted;
        });
        return;
      case "lume.group.mute":
        this.#tabs(() =>
          editGroup((found) => {
            const { muted, ...rest } = found;
            const on = typeof given["on"] === "boolean" ? given["on"] : !muted;
            return on ? { ...rest, muted: true } : rest;
          }),
        );
        return;
      case "lume.tab.rename":
        this.#tabs(() => {
          if (facts) facts.title = typeof given["title"] === "string" ? given["title"] : null;
        });
        return;
      case "lume.group.new":
        this.#tabs(() => {
          if (!facts) return;
          const used = new Set(this.#groups.map((found) => found.color));
          const order: readonly TabColor[] = [
            "blue",
            "pink",
            "yellow",
            "purple",
            "teal",
            "red",
            "green",
            "orange",
          ];
          const id = `g${this.#nextGroup++}`;
          this.#groups.push({ id, color: order.find((color) => !used.has(color)) ?? "blue" });
          facts.group = id;
        });
        return;
      case "lume.group.add":
        this.#tabs(() => {
          if (facts && typeof given["group"] === "string") facts.group = given["group"];
        });
        return;
      case "lume.group.remove":
        this.#tabs(() => {
          if (facts) facts.group = null;
        });
        return;
      case "lume.group.set":
        this.#tabs(() =>
          editGroup((found) => {
            const next: Record<string, unknown> = { ...found };
            for (const key of ["name", "color", "icon", "collapsed", "kept", "muted"]) {
              if (!(key in given)) continue;
              if (given[key] === null) delete next[key];
              else next[key] = given[key];
            }
            return next as unknown as Group;
          }),
        );
        return;
      case "lume.group.toggle":
        this.#tabs(() => editGroup((found) => ({ ...found, collapsed: !found.collapsed })));
        return;
      case "lume.group.ungroup":
        this.#tabs(() => {
          for (const each of this.#facts.values()) if (each.group === group) each.group = null;
        });
        return;
      // A session that is shown again, or ended, is no longer one that no pane shows.
      case "lume.session.reattach":
      case "lume.session.end":
        this.#detached = this.#detached.filter((found) => found.session !== given["session"]);
        return;
      default:
        return;
    }
  }

  /** What the tabs are now, to compare with what they are after a change. */
  #tabsNow(): TabsNow {
    return {
      order: [...this.#order],
      active: this.#active,
      facts: new Map([...this.#facts].map(([tab, facts]) => [tab, { ...facts }])),
      groups: this.#groups.map((group) => ({ ...group })),
    };
  }

  /**
   * Keeps the tabs in their one form, as the core does: a group's tabs stand together
   * where its first tab is, a group without tabs is gone, the groups are listed in the
   * order of their tabs, and the tab on show is never folded away.
   */
  #tidy(): void {
    const known = new Set(this.#groups.map((group) => group.id));
    for (const facts of this.#facts.values()) {
      if (facts.group !== null && !known.has(facts.group)) facts.group = null;
    }
    const seen = new Set<GroupId>();
    const order: TabId[] = [];
    for (const tab of this.#order) {
      const group = this.#facts.get(tab)?.group ?? null;
      if (group === null) {
        order.push(tab);
        continue;
      }
      if (seen.has(group)) continue;
      seen.add(group);
      order.push(...this.#order.filter((other) => this.#facts.get(other)?.group === group));
    }
    this.#order = order;
    this.#groups = [...seen].flatMap((id) => this.#groups.filter((group) => group.id === id));
    const shown = this.#facts.get(this.#active)?.group ?? null;
    this.#groups = this.#groups.map((group) => {
      if (group.id !== shown || !group.collapsed) return group;
      const open: Record<string, unknown> = { ...group };
      delete open["collapsed"];
      return open as unknown as Group;
    });
  }

  /** Changes the tabs, and tells the page what changed, as the core would. */
  #tabs(change: () => void): void {
    const before = this.#tabsNow();
    change();
    this.#tidy();
    const after = this.#tabsNow();
    const changes: LayoutChange[] = [];
    for (const tab of after.order) {
      const was = before.facts.get(tab);
      const is = after.facts.get(tab);
      if (!was || !is) continue;
      if (was.title !== is.title) changes.push({ type: "tab_title", tab, title: is.title });
      if (
        was.color !== is.color ||
        was.locked !== is.locked ||
        was.muted !== is.muted ||
        was.group !== is.group
      ) {
        changes.push({
          type: "tab_state",
          tab,
          color: is.color,
          locked: is.locked,
          ...(is.muted ? { muted: true } : {}),
          group: is.group,
        });
      }
    }
    if (JSON.stringify(before.groups) !== JSON.stringify(after.groups)) {
      changes.push({ type: "groups", window: WINDOW, groups: after.groups });
    }
    if (before.order.join() !== after.order.join() || before.active !== after.active) {
      changes.push({ type: "tabs", window: WINDOW, order: after.order, active: after.active });
    }
    if (changes.length > 0) this.#changed(changes);
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
    const groups = JSON.stringify(this.#groups);
    this.#active = tab;
    this.#focus = focus;
    // A folded group opens when one of its tabs is shown.
    this.#tidy();
    this.#changed([
      ...(JSON.stringify(this.#groups) === groups
        ? []
        : [{ type: "groups" as const, window: WINDOW, groups: this.#groups }]),
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
