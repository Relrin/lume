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

/** The settings a page reads itself. Everything else reaches it as tokens. */
export interface ConfigSettings {
  readonly appearance: {
    readonly "pane-header": "always" | "auto" | "never";
    readonly [key: string]: unknown;
  };
  readonly terminal: {
    readonly "scrollback-lines": number;
    readonly "cursor-style": "block" | "bar" | "underline";
    readonly "cursor-blink": boolean;
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
export type CommandRuns = "core" | "page";

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
  /** The profiles changed: what was found on the machine, or what the user wrote. */
  | ({ readonly type: "profiles_changed" } & ProfileList)
  /** The settings window was asked for again: the page to show, if one was named. */
  | { readonly type: "settings_show"; readonly section: string | null }
  | {
      readonly type: "notice";
      readonly level: "info" | "warning" | "error";
      readonly message: string;
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
    };

/** A control message from the core: a notice, or a request to be answered. */
export type ControlMessage =
  | { readonly notify: Notice }
  | { readonly request: { readonly id: number; readonly body: PageRequestBody } };
