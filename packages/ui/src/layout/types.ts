/**
 * The layout as the core writes it: a workspace's windows, their tabs, the splits and
 * panes of each tab, the panes that float, and the panels docked at a window's edges.
 *
 * These types mirror the core's JSON exactly, field for field and spelling for
 * spelling. The core owns the layout; a page keeps a copy, follows the changes it is
 * sent, and asks for changes by sending intents.
 */

import type { Launcher, TermSize } from "../transport/types.ts";

/** What a whole is divided into: a split's ratios add up to exactly this. */
export const UNITS = 1_000_000;

/** The smallest share of a split one of its children may have. */
export const SHARE_MIN = UNITS / 20;

/** The smallest a floating pane may be along either side, in units. */
export const FLOAT_MIN = UNITS / 10;

/** The side of a pane that floats without being told where, in units. */
export const FLOAT_DEFAULT = (UNITS * 6) / 10;

/** How far each further pane floated without a place is moved from the one before. */
export const FLOAT_CASCADE = UNITS / 40;

/** Names a pane, as in `p12`. */
export type PaneId = string;
/** Names a tab, as in `t3`. */
export type TabId = string;
/** Names a split, as in `s7`. */
export type SplitId = string;
/** Names a window, as in `w1`. */
export type WindowId = string;
/** Names a group of tabs, as in `g4`. */
export type GroupId = string;
/** Names a panel: a dotted name, as in `lume.tabs`. */
export type PanelId = string;

/** A named arrangement of windows. */
export interface Workspace {
  /** Sixteen hexadecimal digits. */
  readonly id: string;
  readonly name: string;
  readonly windows: readonly Window[];
  /** The window the user last worked in. */
  readonly focused_window: WindowId | null;
  readonly scratch: Scratch | null;
  /** The number the next pane, tab, split, group or window is given. */
  readonly next_id: number;
  /** The kept groups that were closed, the most recently closed last. Left out when none. */
  readonly shelf?: readonly Shelved[];
}

/** The scratch terminal: one floating pane a workspace keeps at hand. */
export interface Scratch {
  readonly pane: PaneId;
  /** The pane and where it last floated, while it is put away. */
  readonly parked: Floating | null;
}

/** One window of a workspace. */
export interface Window {
  readonly id: WindowId;
  readonly bounds: Bounds | null;
  readonly dock: DockAreas;
  /** The tabs in the middle of the window. */
  readonly centre: TabGroup;
  /** The panes floating over the tabs, from the one at the back to the one in front. */
  readonly floating: readonly Floating[];
  /** The pane that has the keyboard. */
  readonly focus: PaneId | null;
  /** The panes that had the keyboard before, most recent first. */
  readonly history: readonly PaneId[];
}

/** A window's place and size on the desktop, in logical pixels. */
export interface Bounds {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly maximised: boolean;
  readonly fullscreen: boolean;
}

/** One of the four edges of a window, where panels are docked. */
export type Area = "top" | "left" | "right" | "bottom";

/** The panels at each edge of a window. */
export type DockAreas = Readonly<Record<Area, DockArea>>;

/** The panels docked at one edge of a window. */
export interface DockArea {
  readonly panels: readonly Panel[];
  /** How far the area reaches into the window, in logical pixels. */
  readonly size: number;
  readonly collapsed: boolean;
}

/** Something docked at an edge of a window. */
export interface Panel {
  readonly id: PanelId;
  readonly content: PanelContent;
}

/** A place in the top bar that status items are shown in. */
export type Slot = "topbar.left" | "topbar.center" | "topbar.right";

/** What a panel shows. */
export type PanelContent =
  | { readonly type: "slot"; readonly slot: Slot }
  | { readonly type: "tab_list" }
  | {
      readonly type: "plugin_view";
      readonly plugin: string;
      readonly view: string;
      /** What the view asked to have kept for it; left out when it keeps nothing. */
      readonly state?: unknown;
    };

/** The tabs of a window. */
export interface TabGroup {
  readonly tabs: readonly Tab[];
  /** The tab on show. */
  readonly active: TabId | null;
  /**
   * The groups the tabs are gathered in, in the order their first tabs come. A group's
   * tabs stand together. Left out when there are none.
   */
  readonly groups?: readonly Group[];
}

/** One tab: a tree of splits and panes. */
export interface Tab {
  readonly id: TabId;
  /** The name the user gave the tab. */
  readonly title: string | null;
  readonly root: Node;
  /** The pane that fills the tab for now, hiding the others. */
  readonly zoomed: PaneId | null;
  /** The pane of this tab that last had the keyboard. */
  readonly last_focus: PaneId | null;
  /** The colour the user gave the tab. Left out when none. */
  readonly color?: TabColor;
  /** Locked against being closed. Left out when not. */
  readonly locked?: boolean;
  /** Muted: what runs in it does not ask for attention. Left out when not. */
  readonly muted?: boolean;
  /** The group the tab is in. Left out when none. */
  readonly group?: GroupId;
}

/** The colours a tab or a group can be given by name; the theme says how each looks. */
export type NamedTabColor =
  "red" | "orange" | "yellow" | "green" | "teal" | "blue" | "purple" | "pink";

/** A colour of a tab or a group: one of the theme's, or one of the user's own as `#rrggbb`. */
export type TabColor = NamedTabColor | `#${string}`;

/** Every named colour, in the order a tab's menu offers them. */
export const TAB_COLORS: readonly NamedTabColor[] = [
  "red",
  "orange",
  "yellow",
  "green",
  "teal",
  "blue",
  "purple",
  "pink",
];

/** The pictures a group's label can show. */
export type GroupIcon =
  "bug" | "flame" | "rocket" | "flask" | "wrench" | "layers" | "server" | "book";

/** Every group picture, in the order the group editor offers them. */
export const GROUP_ICONS: readonly GroupIcon[] = [
  "bug",
  "flame",
  "rocket",
  "flask",
  "wrench",
  "layers",
  "server",
  "book",
];

/** Tabs of a window gathered under one name and colour. */
export interface Group {
  readonly id: GroupId;
  /** The name the user gave the group. Left out when none. */
  readonly name?: string;
  readonly color: TabColor;
  readonly icon?: GroupIcon;
  /** Folded: only the label shows. The tab on show is never in a folded group. */
  readonly collapsed?: boolean;
  /** Put away when closed, to be opened again, instead of being forgotten. */
  readonly kept?: boolean;
  /** Muted: none of its tabs asks for attention, whatever each says of itself. */
  readonly muted?: boolean;
}

/** A kept group that was closed, with its tabs as they were. */
export interface Shelved {
  readonly group: Group;
  readonly tabs: readonly Tab[];
}

/** A node of a tab's tree. */
export type Node = ({ readonly type: "split" } & Split) | ({ readonly type: "pane" } & Pane);

/** A split as a node of a tree. */
export type SplitNode = Extract<Node, { type: "split" }>;

/** A pane as a node of a tree. */
export type PaneNode = Extract<Node, { type: "pane" }>;

/** The direction a split lays its children out in: side by side, or one above the other. */
export type Axis = "row" | "column";

/** A node that divides its space among two or more children. */
export interface Split {
  readonly id: SplitId;
  readonly axis: Axis;
  /** Each child's share, in units, adding up to exactly {@link UNITS}. */
  readonly ratios: readonly number[];
  readonly children: readonly Node[];
}

/** A leaf of a tab's tree, or a floating pane: one place that shows one thing. */
export interface Pane {
  readonly id: PaneId;
  /** The name the user gave the pane. */
  readonly title: string | null;
  /**
   * The picture the pane was given to be known by: the name of a tile, or `image:` and
   * the key of a plugin's picture. Left out when it was given none.
   */
  readonly icon?: string | null;
  readonly content: PaneContent;
}

/** A pane that shows a terminal session, and what is needed to start the session again. */
export interface TerminalPane {
  /** The backend the session runs on. Without one, it is the application's default. */
  readonly backend: string | null;
  /** The running session, if the pane has one. */
  readonly session: string | null;
  readonly profile: string | null;
  readonly launcher: Launcher | null;
  /** The directory the session was started in. */
  readonly cwd: string | null;
  readonly env: readonly (readonly [string, string])[];
}

/** What a pane shows. */
export type PaneContent =
  | ({ readonly type: "terminal" } & TerminalPane)
  | {
      readonly type: "plugin_view";
      readonly plugin: string;
      readonly view: string;
      readonly state: unknown;
    };

/** A pane that floats over a window's tabs. */
export interface Floating {
  readonly pane: Pane;
  readonly rect: UnitRect;
  /**
   * Set while the pane is put away at the corner of its window, and says where it goes
   * when it is brought back: to float where it floated, or among the tabs again.
   */
  readonly minimised: Minimised | null;
}

/** Where a pane that is put away goes when it is brought back. */
export type Minimised = "floating" | "tab";

/** A rectangle within a whole that is {@link UNITS} wide and high. */
export interface UnitRect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

/** A direction on the screen. */
export type Dir = "left" | "right" | "up" | "down";

/** How the session of a new terminal pane is to be started. Everything may be left out. */
export interface SpawnRequest {
  readonly backend?: string | null;
  readonly profile?: string | null;
  readonly launcher?: Launcher | null;
  readonly cwd?: string | null;
  readonly env?: readonly (readonly [string, string])[];
  /** The grid the pane will have, if whoever asks can tell. */
  readonly size?: TermSize | null;
}

/** What a new pane shows. */
export type NewContent =
  | ({ readonly type: "terminal" } & SpawnRequest)
  | {
      readonly type: "plugin_view";
      readonly plugin: string;
      readonly view: string;
      readonly state?: unknown;
    };

/** Which pane is to have the keyboard. */
export type FocusTarget =
  | { readonly type: "pane"; readonly pane: PaneId }
  | { readonly type: "next"; readonly window: WindowId }
  | { readonly type: "previous"; readonly window: WindowId }
  | { readonly type: "back"; readonly window: WindowId };

/** Which tab is to be shown. */
export type TabTarget =
  | { readonly type: "id"; readonly tab: TabId }
  | { readonly type: "index"; readonly window: WindowId; readonly index: number }
  | { readonly type: "next"; readonly window: WindowId }
  | { readonly type: "previous"; readonly window: WindowId };

/** Where a pane is to go. */
export type Place =
  | { readonly type: "beside"; readonly pane: PaneId; readonly edge: Dir }
  | { readonly type: "new_tab"; readonly window: WindowId; readonly index?: number | null };

/**
 * A change asked of the layout. The last four record what the application alone
 * knows; the core refuses them from a page.
 */
export type Intent =
  | {
      readonly type: "split";
      readonly pane: PaneId;
      readonly dir: Dir;
      /** The new pane's share of the divided space, in units. */
      readonly ratio: number;
      readonly content: NewContent;
    }
  | { readonly type: "close"; readonly pane: PaneId }
  | { readonly type: "focus"; readonly target: FocusTarget }
  | { readonly type: "focus_dir"; readonly window: WindowId; readonly dir: Dir }
  | { readonly type: "move"; readonly pane: PaneId; readonly to: Place }
  | { readonly type: "swap"; readonly a: PaneId; readonly b: PaneId }
  | { readonly type: "resize"; readonly split: SplitId; readonly ratios: readonly number[] }
  | { readonly type: "zoom"; readonly pane: PaneId; readonly on: boolean }
  | { readonly type: "float"; readonly pane: PaneId; readonly rect?: UnitRect | null }
  | { readonly type: "float_move"; readonly pane: PaneId; readonly rect: UnitRect }
  | { readonly type: "minimise"; readonly pane: PaneId; readonly on: boolean }
  | { readonly type: "dock"; readonly pane: PaneId; readonly to?: Place | null }
  | { readonly type: "pane_rename"; readonly pane: PaneId; readonly title?: string | null }
  | { readonly type: "pane_icon"; readonly pane: PaneId; readonly icon?: string | null }
  | {
      readonly type: "tab_new";
      readonly window: WindowId;
      readonly content: NewContent;
      readonly index?: number | null;
      readonly activate?: boolean;
      /** Opens the tab after the last tab of this group, in it. */
      readonly group?: GroupId | null;
    }
  | { readonly type: "tab_close"; readonly tab: TabId }
  | { readonly type: "tabs_close"; readonly tabs: readonly TabId[] }
  | { readonly type: "tab_move"; readonly tab: TabId; readonly index: number }
  | {
      readonly type: "tab_place";
      readonly tab: TabId;
      readonly index: number;
      readonly group?: GroupId | null;
    }
  | { readonly type: "tab_color"; readonly tab: TabId; readonly color?: TabColor | null }
  | { readonly type: "tab_lock"; readonly tab: TabId; readonly on: boolean }
  | { readonly type: "tab_mute"; readonly tab: TabId; readonly on: boolean }
  | { readonly type: "tab_merge"; readonly tab: TabId; readonly into: TabId }
  | { readonly type: "tab_rename"; readonly tab: TabId; readonly title?: string | null }
  | { readonly type: "tab_activate"; readonly target: TabTarget }
  | { readonly type: "group_new"; readonly tab: TabId }
  | {
      readonly type: "group_set";
      readonly group: GroupId;
      /** Null takes the name away; left out keeps it. */
      readonly name?: string | null;
      readonly color?: TabColor;
      /** Null takes the picture away; left out keeps it. */
      readonly icon?: GroupIcon | null;
      readonly collapsed?: boolean;
      readonly kept?: boolean;
      readonly muted?: boolean;
    }
  | { readonly type: "group_move"; readonly group: GroupId; readonly index: number }
  | { readonly type: "group_ungroup"; readonly group: GroupId }
  | { readonly type: "group_close"; readonly group: GroupId }
  | { readonly type: "group_reopen"; readonly group: GroupId; readonly window: WindowId }
  | { readonly type: "group_forget"; readonly group: GroupId }
  | {
      readonly type: "panel_move";
      readonly window: WindowId;
      readonly panel: PanelId;
      readonly area: Area;
      readonly index: number;
    }
  | {
      readonly type: "panel_toggle";
      readonly window: WindowId;
      readonly area: Area;
      readonly on?: boolean | null;
    }
  | {
      readonly type: "panel_resize";
      readonly window: WindowId;
      readonly area: Area;
      readonly size: number;
    }
  | {
      readonly type: "scratch_show";
      readonly window: WindowId;
      readonly content?: NewContent | null;
    }
  | { readonly type: "scratch_hide" }
  | { readonly type: "window_new"; readonly content: NewContent; readonly bounds?: Bounds | null }
  | { readonly type: "window_close"; readonly window: WindowId }
  | {
      readonly type: "session_set";
      readonly pane: PaneId;
      readonly session: string | null;
      readonly backend?: string | null;
      readonly cwd?: string | null;
    }
  | {
      readonly type: "panel_set";
      readonly window: WindowId;
      readonly panel: PanelId;
      readonly content: PanelContent;
      readonly area: Area | null;
      readonly index: number;
    }
  | { readonly type: "view_state"; readonly pane: PaneId; readonly state: unknown }
  | {
      readonly type: "panel_state";
      readonly window: WindowId;
      readonly panel: PanelId;
      readonly state: unknown;
    }
  | { readonly type: "window_bounds"; readonly window: WindowId; readonly bounds: Bounds }
  | { readonly type: "window_focus"; readonly window: WindowId };

/** One difference between a layout before an intent and after it. */
export type LayoutChange =
  | { readonly type: "window_opened"; readonly window: Window }
  | { readonly type: "window_bounds"; readonly window: WindowId; readonly bounds: Bounds }
  | {
      readonly type: "tab_added";
      readonly window: WindowId;
      readonly tab: Tab;
      readonly index: number;
    }
  | { readonly type: "tab_removed"; readonly tab: TabId }
  | {
      readonly type: "tabs";
      readonly window: WindowId;
      readonly order: readonly TabId[];
      readonly active: TabId | null;
    }
  | { readonly type: "tab_title"; readonly tab: TabId; readonly title: string | null }
  | {
      readonly type: "tab_state";
      readonly tab: TabId;
      readonly color: TabColor | null;
      readonly locked: boolean;
      /** Left out while the tab is not muted. */
      readonly muted?: boolean;
      readonly group: GroupId | null;
    }
  | { readonly type: "groups"; readonly window: WindowId; readonly groups: readonly Group[] }
  | {
      readonly type: "tab_tree";
      readonly tab: TabId;
      readonly root: Node;
      readonly zoomed: PaneId | null;
    }
  | { readonly type: "ratios"; readonly split: SplitId; readonly ratios: readonly number[] }
  | { readonly type: "pane"; readonly pane: Pane }
  | {
      readonly type: "floating";
      readonly window: WindowId;
      readonly floating: readonly Floating[];
    }
  | { readonly type: "float_rect"; readonly pane: PaneId; readonly rect: UnitRect }
  | {
      readonly type: "dock";
      readonly window: WindowId;
      readonly area: Area;
      readonly dock: DockArea;
    }
  | {
      readonly type: "dock_size";
      readonly window: WindowId;
      readonly area: Area;
      readonly size: number;
    }
  | { readonly type: "focus"; readonly window: WindowId; readonly pane: PaneId | null }
  | { readonly type: "window_closed"; readonly window: WindowId }
  | { readonly type: "scratch"; readonly scratch: Scratch | null }
  | { readonly type: "window_focused"; readonly window: WindowId | null }
  | { readonly type: "workspace_name"; readonly name: string }
  | { readonly type: "shelf"; readonly shelf: readonly Shelved[] }
  | { readonly type: "pane_closed"; readonly pane: PaneId; readonly content: PaneContent };

/** What an intent made. */
export interface Created {
  readonly pane: PaneId | null;
  readonly tab: TabId | null;
  readonly window: WindowId | null;
  /** The group a `group_new` made. Left out when none. */
  readonly group?: GroupId;
}
