/**
 * What the page and the core say to each other besides a session's bytes.
 *
 * The core owns the layout, the configuration and the commands. A page asks for each
 * as a whole when it starts, and is then told what changes: notices arrive as control
 * messages on the carrier, in the order the changes happened. The core in turn asks
 * the page what only the page knows, such as what is on a pane's screen; those
 * requests arrive the same way and are answered through a command.
 *
 * The spellings here are the wire's: field names are as the core writes them.
 */

import type { Tokens } from "@lume/tokens";

import type {
  Created,
  Intent,
  LayoutChange,
  PaneId,
  WindowId,
  Workspace,
} from "../layout/types.ts";
import type { Launcher, TermSize } from "../transport/types.ts";

/** Whether a look is a light or a dark one. */
export type Scheme = "light" | "dark";

/** The platform's name as the configuration spells it. */
export type ConfigPlatform = "windows" | "macos" | "linux";

/** Something wrong in the user's files that was passed over. */
export interface ConfigProblem {
  readonly kind: string;
  /** The setting's path, such as `font.size`. Empty for a problem with a whole file. */
  readonly path: string;
  readonly file: string | null;
  /** Counted from one; nought when the problem has no place in the file. */
  readonly line: number;
  readonly column: number;
  readonly message: string;
  readonly suggestion: string | null;
}

/** One key and the command it runs. */
export interface KeyBinding {
  /** The chords to press one after another, each spelt as in `ctrl+shift+t`. */
  readonly keys: readonly string[];
  readonly command: string;
  /** What the command is given. `null` when it is given nothing. */
  readonly args: unknown;
  /** Where the binding holds: context names, each negated by a leading `!`. */
  readonly when: readonly string[];
  readonly source: unknown;
}

/** Something a plugin brings, by the id of the plugin as installed and its own name. */
export type Contributed = `${string}/${string}`;

/** How the mark of the tab on show moves from one tab to another. */
export type TabHighlight = "instant" | "glide" | "stretch" | "spring" | "glow";

/** What the cursor leaves behind when it jumps a distance or moves to another pane. */
export type CursorTrail = "none" | "warp" | "sweep" | "comet";

/** What the cursor gives off when it changes shape or moves to another pane. */
export type CursorPulse = "none" | "burst" | "ripple" | "box-burst" | "box-ripple";

/** What a notice offers to do about what it says: a command, run when its button is pressed. */
export interface NoticeAction {
  readonly label: string;
  readonly command: string;
  readonly args?: unknown;
}

/** The settings a page reads itself. Everything else reaches it as tokens. */
export interface ConfigSettings {
  readonly appearance: {
    readonly "pane-header": "always" | "auto" | "never";
    /** Which program tiles are drawn in colour. A core that does not say means all of them. */
    readonly "tile-color"?: "always" | "active-pane" | "active-tab" | "never";
    /** How the mark of the tab on show moves to another. A core that does not say means glow. */
    readonly "tab-highlight"?: TabHighlight | Contributed;
    /** The plugins' effects chosen for places in the window, by place. */
    readonly motion?: Readonly<Record<string, Contributed | undefined>>;
    readonly [key: string]: unknown;
  };
  readonly terminal: {
    readonly "scrollback-lines": number;
    /** Keep only the newest lines in a window. A core that does not say means off. */
    readonly "history-paging"?: boolean;
    /** How many lines a window keeps then. A core that does not say means 1000. */
    readonly "window-history-lines"?: number;
    /** How long fetched lines stay at the newest output. A core that does not say means 60. */
    readonly "history-release-seconds"?: number;
    /** Let panes out of sight let go of their screen. A core that does not say means off. */
    readonly "hidden-release"?: boolean;
    /** After how many seconds out of sight. A core that does not say means 30. */
    readonly "hidden-release-seconds"?: number;
    readonly "cursor-style": "block" | "bar" | "underline";
    readonly "cursor-blink": boolean;
    /** What the cursor leaves behind when it jumps. A core that does not say means none. */
    readonly "cursor-trail"?: CursorTrail | Contributed;
    /** What the cursor gives off when it changes shape. A core that does not say means none. */
    readonly "cursor-pulse"?: CursorPulse | Contributed;
    /** How long both last against their own length. A core that does not say means 1. */
    readonly "cursor-effect-length"?: number;
    readonly [key: string]: unknown;
  };
  readonly layout: {
    readonly "tab-placement": "top" | "left" | "right";
    readonly [key: string]: unknown;
  };
  readonly renderer: {
    readonly tier: "auto" | "webgpu" | "webgl2" | "canvas2d";
    readonly [key: string]: unknown;
  };
  readonly [section: string]: unknown;
}

/** The configuration in use, as `config_view` returns it and `config_changed` carries it. */
export interface ConfigView {
  /** Higher than that of every view before this one. */
  readonly revision: number;
  readonly platform: ConfigPlatform;
  /** The colour scheme of the operating system. */
  readonly scheme: Scheme;
  /** The theme the tokens were resolved from. */
  readonly theme: { readonly id: string; readonly scheme: Scheme };
  readonly config: ConfigSettings;
  /** Where each setting came from, by the setting's path. */
  readonly sources: Readonly<Record<string, unknown>>;
  readonly problems: readonly ConfigProblem[];
  /** Every design token's value. */
  readonly tokens: Tokens;
  readonly keymap: { readonly bindings: readonly KeyBinding[] };
  /**
   * Who draws the window's title bar: the page, with its own buttons in the top bar,
   * or the window system. Already decided for the platform.
   */
  readonly title_bar: TitleBar;
  /**
   * Counts the schemas of the settings there have been: it changes when plugins add to
   * the settings or take from them. A core that does not say has one schema only.
   */
  readonly schema_revision?: number;
}

/** Who draws a window's title bar. */
export type TitleBar = "custom" | "system";

/** What a window is doing on the desktop, as far as the page shows it. */
export interface WindowState {
  readonly maximised: boolean;
  readonly fullscreen: boolean;
  /** Whether the window is the one the user is working in. */
  readonly focused: boolean;
}

/** What a page may ask to have done to its own window. */
export type WindowAction = "minimise" | "maximise" | "close";

/** The workspaces there are, as `workspaces_list` returns them. */
export interface WorkspaceList {
  /** The name of the workspace on show. */
  readonly active: string;
  readonly workspaces: readonly {
    readonly name: string;
    /** The folder new terminals of the workspace start in, if it has one. */
    readonly root: string | null;
  }[];
}

/** A theme there is to choose, as `themes_list` describes it. */
export interface ThemeEntry {
  /** What the configuration calls the theme. */
  readonly id: string;
  /** What a person calls it. */
  readonly name: string;
  readonly scheme: Scheme;
  /** Whether the theme comes with the application, not from the user's folder. */
  readonly builtin: boolean;
  /** The plugin it comes from, by the id it is installed under, if it does. */
  readonly plugin?: string;
}

/** A picture a plugin's thing shows: one of the page's, by name, or one of the plugins'. */
export type IconRef = { readonly glyph: string } | { readonly image: string };

/**
 * A picture of a plugin's: the hash it is fetched by when it is first drawn, and
 * whether it takes the colour around it.
 */
export interface PluginImage {
  readonly hash: string;
  readonly tint: boolean;
}

/** One node of a menu, in its place: the application's own (`lume:<key>`) or a plugin's. */
export interface MenuNodeData {
  readonly kind: "section" | "item" | "submenu";
  readonly key: string;
  /** The plugin that adds it, by the id it is installed under. */
  readonly plugin?: string;
  readonly title?: string;
  /** The command an item runs, by its full id. */
  readonly command?: string;
  readonly args?: unknown;
  readonly icon?: IconRef;
  /** Conditions it is shown under, each perhaps after `!`; all must hold. */
  readonly when?: readonly string[];
  readonly children?: readonly MenuNodeData[];
}

/** A plugin's status item, as its manifest says it, in its place. */
export interface StatusDef {
  readonly id: string;
  readonly plugin: string;
  readonly slot: string;
  readonly kind: string;
  readonly order: number;
  readonly label?: string;
  readonly icon?: IconRef;
  readonly tooltip?: string;
  readonly command?: string;
  /** The key of the submenu in `menu.status` it opens. */
  readonly menu?: string;
  readonly when?: readonly string[];
  /** The plugin's switch that must be on for it to be shown, and the switch's default. */
  readonly shown_when?: { readonly key: string; readonly default: boolean };
  /**
   * The plugin's setting that chooses which of the bars' places it is in, and the places
   * it may choose. Until the user sets it, the item is in `slot`.
   */
  readonly slot_setting?: { readonly key: string; readonly slots: readonly string[] };
}

/** A plugin's panel, as its manifest says it: a list, or its own page (`frame`). */
export interface PanelDef {
  readonly id: string;
  readonly plugin: string;
  readonly view: string;
  readonly title: string;
  readonly kind: string;
  readonly area: string;
  readonly order: number;
  readonly icon?: IconRef;
  /** The page a `frame` panel shows, as a path in the plugin's package. */
  readonly entry?: string;
}

/** A view a plugin draws in a frame, which a pane can show. */
export interface PaneViewDef {
  /** `<plugin>.<name>`. */
  readonly id: string;
  readonly plugin: string;
  /** The name its manifest gives it, which the layout calls its view. */
  readonly view: string;
  readonly title: string;
  /** The page it shows, as a path in the plugin's package. */
  readonly entry: string;
  readonly persist: boolean;
  readonly icon?: IconRef;
}

/** Files or addresses one of a plugin's pane views opens. */
export interface OpenerDef {
  readonly plugin: string;
  readonly view: string;
  readonly title: string;
  /** File name patterns, lower case. */
  readonly patterns: readonly string[];
  readonly schemes: readonly string[];
}

/** An easing as the core checked it. */
export type MotionEasing =
  | { readonly css: string }
  | { readonly token: string }
  | { readonly spring: readonly [number, number] };

/** One keyframe of a plugin's effect, as the browser's animations take it. */
export type MotionKeyframe = Readonly<Record<string, string | number>>;

/** An effect a plugin brings for one of the window's places. */
export interface MotionDef {
  /** `<plugin>/<name>`, as the settings choose it. */
  readonly id: string;
  readonly plugin: string;
  readonly title: string;
  readonly slot: string;
  readonly spec:
    | {
        readonly slot: string;
        readonly keyframes: readonly MotionKeyframe[];
        readonly duration: number;
        readonly delay: number;
        readonly easing: MotionEasing;
      }
    | {
        readonly slot: "tab.highlight";
        readonly highlight: {
          readonly shape: "pill" | "underline" | "glow";
          readonly lead: readonly [number, number];
          readonly trail: readonly [number, number];
          readonly slide: "spring" | { readonly duration: number; readonly easing: MotionEasing };
          readonly enter: {
            readonly keyframes: readonly MotionKeyframe[];
            readonly duration?: number;
            readonly spring?: readonly [number, number];
          };
          readonly renamed: readonly MotionKeyframe[] | null;
        };
      };
}

/** A cursor effect a plugin's shader draws. */
export interface CursorEffectDef {
  /** `<plugin>/<name>`, as the settings choose it. */
  readonly id: string;
  readonly plugin: string;
  readonly title: string;
  readonly kind: "trail" | "pulse";
  readonly duration: number;
  readonly reach: number;
  /**
   * The hash the shader is fetched by when the effect is chosen: the GLSL of
   * `vec4 lume_effect(vec2 pixel)` and its helpers.
   */
  readonly hash: string;
}

/** A plugin that is installed, as a window is told of it. */
export interface PluginEntry {
  readonly id: string;
  readonly name: string;
  readonly version: string;
  /** Whether it can be used: installed, turned on, its package as installed. */
  readonly ready: boolean;
  /** What went wrong with it last, if anything did. */
  readonly problem: string | null;
  /** Whether it is a folder used where it lies, for a plugin being worked on. */
  readonly dev?: boolean;
}

/** One row of a list panel, as a plugin gave it. */
export interface PanelRowData {
  readonly id: string;
  readonly label: string;
  readonly detail?: string;
  readonly icon?: string;
  readonly badge?: string;
  readonly tone?: "neutral" | "info" | "success" | "warning" | "danger";
  readonly children?: readonly PanelRowData[];
  /** Whether its rows are given only when it is opened. */
  readonly lazy?: boolean;
  /** Whether a row that is `lazy` has been given its rows. */
  readonly loaded?: boolean;
  readonly expanded?: boolean;
  readonly actions?: readonly {
    readonly command: string;
    readonly title: string;
    readonly icon?: string;
    readonly args?: unknown;
  }[];
}

/** What the plugins add to a window, as `ui_contributions` returns it. */
export interface UiContributions {
  readonly revision: number;
  readonly plugins: readonly PluginEntry[];
  readonly menus: Readonly<Record<string, readonly MenuNodeData[]>>;
  readonly status: readonly StatusDef[];
  readonly panels: readonly PanelDef[];
  readonly pane_views: readonly PaneViewDef[];
  readonly openers: readonly OpenerDef[];
  readonly images: Readonly<Record<string, PluginImage>>;
  readonly motions: readonly MotionDef[];
  readonly cursor_effects: readonly CursorEffectDef[];
  /** What the plugins' status items show now. */
  readonly values: readonly {
    readonly plugin: string;
    readonly item: string;
    readonly pane: string | null;
    readonly value: Readonly<Record<string, unknown>>;
  }[];
  /** What the list panels this window has on screen show now, by panel. */
  readonly rows: Readonly<Record<string, readonly PanelRowData[]>>;
  /** The conditions plugins set, by name (`plugin:<id>.<key>`). */
  readonly contexts: Readonly<Record<string, boolean>>;
}

/** A shell found on this machine, as `shells_list` describes it. */
export interface ShellEntry {
  /** What a person calls the shell. */
  readonly name: string;
  /** What the configuration names to start it. */
  readonly program: string;
}

/** A way to start a terminal, as `profiles_list` describes it. */
export interface Profile {
  /** What commands and the configuration name it by. */
  readonly id: string;
  /** What a person calls it. */
  readonly name: string;
  readonly icon?: string;
  readonly launcher: Launcher;
  readonly cwd?: string;
  readonly theme?: string;
  readonly font_size?: number;
  /** Left out of lists; it can still be named. */
  readonly hidden: boolean;
  /** Whether the application made it for something it found, or the user wrote it. */
  readonly origin: "generated" | "user";
  /** A line about it: where it was found, what state it is in. */
  readonly detail?: string;
}

/** A theme's look as a profile that names it is drawn with. */
export interface ProfileLook {
  readonly scheme: Scheme;
  readonly tokens: Tokens;
}

/** The profiles there are, as `profiles_list` returns them. */
export interface ProfileList {
  readonly revision: number;
  readonly profiles: readonly Profile[];
  /** The id of the profile new terminals use, if one is set. */
  readonly default: string | null;
  /** Why something that might be expected is not among them. */
  readonly notes: readonly string[];
  /** Whether the machine has been looked at yet. Before that only the user's own are listed. */
  readonly looked: boolean;
  /** The look of each theme a profile names, by the theme's id. */
  readonly looks?: Readonly<Record<string, ProfileLook>>;
}

/**
 * A session the session host runs that no pane of any workspace shows, as
 * `sessions_detached` describes it.
 */
export interface DetachedSession {
  readonly backend: string;
  readonly session: string;
  /** The program the session was started with. */
  readonly program: string;
  /** The title the program last gave the terminal, if it gave one. */
  readonly title: string | null;
  /** The directory its shell last said it was in, if it said. */
  readonly cwd: string | null;
  /** When the session started, in milliseconds since the Unix epoch. */
  readonly created_unix_ms: number;
  /** Whether its program has ended. Its last screen can still be shown. */
  readonly exited: boolean;
  readonly pid: number | null;
}

/** What `sessions_detached` returns. */
export interface DetachedSessions {
  readonly sessions: readonly DetachedSession[];
}

/** What a page knows of the machine it runs on. */
export interface Machine {
  /** The machine's name, as the system gives it, where it does. */
  readonly name: string | null;
  /** The user's home folder. */
  readonly home: string | null;
}

/** The layout as `layout_view` returns it. */
export interface LayoutView {
  /** The window this page is. */
  readonly window: WindowId;
  /** Counts how many times the layout was replaced as a whole. */
  readonly epoch: number;
  /** Counts the changes in this epoch. */
  readonly revision: number;
  readonly workspace: Workspace;
  /** What the page needs of this machine. Sent with the view the page asks for. */
  readonly machine?: Machine;
  /** Whether the session host is connected. Sent with the view the page asks for. */
  readonly host?: { readonly connected: boolean };
}

/** What `layout_apply` returns. */
export interface LayoutApplied {
  readonly epoch: number;
  readonly revision: number;
  readonly created: Created;
}

/** A pane whose session the page asks the core to start, at the grid the pane has. */
export interface PaneStart {
  readonly pane: PaneId;
  readonly size: TermSize;
}

/** What `sessions_start` returns: a session for each pane, or why it has none. */
export interface SessionsStarted {
  readonly started: readonly {
    readonly pane: PaneId;
    readonly backend: string;
    readonly session: string;
  }[];
  readonly failed: readonly {
    readonly pane: PaneId;
    readonly code: string;
    readonly message: string;
  }[];
}

/** Where a command is carried out. */
export type CommandRuns = "core" | "page" | "plugin";

/** A command as `commands_list` describes it. */
export interface CommandSpec {
  /** Dotted and lower case, such as `lume.pane.split-right`. */
  readonly id: string;
  readonly title: string;
  readonly category: string;
  readonly runs: CommandRuns;
  /** Whether the command is offered in the palette. */
  readonly palette: boolean;
  /** A JSON Schema of the command's arguments, if it takes any. */
  readonly args?: unknown;
}

/**
 * What the application is, as `about_info` says it: versions, and words for kinds of
 * things. Nothing in it names the machine, the user, a host or a folder.
 */
export interface AboutInfo {
  readonly appVersion: string;
  readonly os: string;
  readonly arch: string;
  /** `WebView2`, `WKWebView` or `WebKitGTK`. */
  readonly engine: string;
  readonly engineVersion: string | null;
  /** The version of the protocol the application speaks to a session host. */
  readonly proto: number;
  /** The session host in use, if the application has one. */
  readonly host: {
    readonly version: string;
    readonly proto: number;
    /** Whether it runs on its own, as opposed to inside the application. */
    readonly daemon: boolean;
    readonly connected: boolean;
  } | null;
  /** The version of the Rex server installed on this machine, if one is. */
  readonly rexVersion: string | null;
}

/** What the core tells a page without being asked. */
export type Notice =
  | {
      readonly type: "layout_changed";
      readonly epoch: number;
      readonly revision: number;
      readonly changes: readonly LayoutChange[];
    }
  | ({ readonly type: "layout_reset" } & LayoutView)
  | {
      readonly type: "config_changed";
      readonly revision: number;
      /** The paths of the settings that differ from the view before. */
      readonly changed: readonly string[];
      readonly view: ConfigView;
    }
  | { readonly type: "config_error"; readonly problems: readonly ConfigProblem[] }
  | ({ readonly type: "window_state" } & WindowState)
  /**
   * The window was asked to close while it must ask first. The page asks, and closes
   * the window itself if the answer is yes.
   */
  | { readonly type: "close_requested" }
  /** The window was kept ready in the background and has been shown again. */
  | { readonly type: "window_shown" }
  /**
   * A command that would end programs was run from this page. The page asks, and runs
   * `command` with `args` as given if the answer is yes.
   */
  | {
      readonly type: "confirm_requested";
      readonly title: string;
      readonly message: string;
      /** The label of the button that goes ahead. */
      readonly confirm: string;
      readonly command: string;
      readonly args: Readonly<Record<string, unknown>>;
    }
  /** The profiles changed: what was found on the machine, or what the user wrote. */
  | ({ readonly type: "profiles_changed" } & ProfileList)
  /** Commands were added or taken away, as plugins come and go. */
  | { readonly type: "commands_changed" }
  /** The plugins that are installed changed. */
  | { readonly type: "plugins_changed" }
  /** What the plugins add to the windows changed: the page fetches it again. */
  | { readonly type: "contributions_changed"; readonly revision: number }
  /** The files of a plugin being worked on changed: its open views load again. */
  | { readonly type: "plugin_view_reload"; readonly plugin: string }
  /** A plugin's code sent one of this page's open views an event. */
  | {
      readonly type: "view_event";
      /** The page's name for the view. */
      readonly view: string;
      readonly event: string;
      readonly payload: unknown;
    }
  /**
   * What a plugin's code showed since the window was last told, all of it together:
   * status items' values (null where one shows nothing any more), the rows of the
   * panels this window has on screen (null where one shows none), and the conditions
   * it set, by name (`plugin:<id>.<key>`). The page takes it in in one step.
   */
  | {
      readonly type: "plugin_frame";
      readonly plugin: string;
      readonly statuses: readonly {
        readonly item: string;
        readonly pane: string | null;
        readonly value: Readonly<Record<string, unknown>> | null;
      }[];
      readonly panels: Readonly<Record<string, readonly PanelRowData[] | null>>;
      readonly contexts: Readonly<Record<string, boolean>>;
    }
  /** How a plugin fares: what went wrong with its code, or nothing when all is well. */
  | { readonly type: "plugin_state"; readonly plugin: string; readonly problem: string | null }
  /** The settings window was asked for again: the page to show, if one was named. */
  | { readonly type: "settings_show"; readonly section: string | null }
  /** The connection to the session host was lost, or made again. */
  | { readonly type: "host_link"; readonly connected: boolean }
  | {
      readonly type: "notice";
      readonly level: "info" | "warning" | "error";
      readonly message: string;
      /** What the user may do about it, as a button beside the message. */
      readonly action?: NoticeAction;
    };

/**
 * What the core may ask a page. A request about a screen names a pane, a session, or
 * neither, which means the pane that has the keyboard.
 */
export type PageRequestBody =
  /** The grid the pane an intent would make is going to have. */
  | { readonly type: "estimate"; readonly intent: Intent }
  /** Run a command that runs in the page. */
  | { readonly type: "command"; readonly id: string; readonly args?: unknown }
  | { readonly type: "screen"; readonly pane?: PaneId; readonly session?: string }
  | {
      readonly type: "wait_text";
      readonly pane?: PaneId;
      readonly session?: string;
      readonly text: string;
      readonly timeout_ms: number;
    }
  | {
      readonly type: "resize";
      readonly pane?: PaneId;
      readonly session?: string;
      readonly size: TermSize;
    }
  /** Put a plugin's question to the user: drawn by the page, under the plugin's name. */
  | { readonly type: "plugin_ask"; readonly ask: unknown };

/** A control message from the core: a notice, or a request to be answered. */
export type ControlMessage =
  | { readonly notify: Notice }
  | { readonly request: { readonly id: number; readonly body: PageRequestBody } };
