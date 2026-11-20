/**
 * What the window's tabs show that only this page knows: which tab is being renamed,
 * which menu or editor is open over the tabs, and what is being dragged among them.
 *
 * None of it is the layout's. The layout says what the tabs are; this says what the
 * user is doing with them right now, and is gone when the page reloads.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { GroupId, PaneId, TabId } from "../layout/types.ts";

/** Where a menu or an editor opens from: the box of what opened it, in the window. */
export interface Anchor {
  readonly left: number;
  readonly top: number;
  readonly bottom: number;
}

/** The box of an element as an anchor. */
export function anchorOf(element: Element): Anchor {
  const box = element.getBoundingClientRect();
  return { left: box.left, top: box.top, bottom: box.bottom };
}

/** A menu or an editor open over the tabs. */
export type TabPopup =
  | {
      /** The menu of one tab. */
      readonly kind: "tab";
      readonly tab: TabId;
      readonly anchor: Anchor;
      /** Whether it was opened from the keyboard, and so takes it. */
      readonly keyboard: boolean;
    }
  | {
      /** The editor of one group. */
      readonly kind: "group";
      readonly group: GroupId;
      readonly anchor: Anchor;
      /** Whether its name field takes the keyboard, as it does for a group just made. */
      readonly focusName: boolean;
    }
  | {
      /** The menu of one pane, opened from its header. */
      readonly kind: "pane";
      readonly pane: PaneId;
      readonly anchor: Anchor;
      readonly keyboard: boolean;
    };

/** Which side of what it is let go over a dragged tab or group lands on. */
export type DropSide = "before" | "after";

/**
 * A drag among the tabs. What is dragged and what it would land beside are named as
 * the row names them: a tab by its id, a group's label by `L` and the group's id.
 */
export interface TabDrag {
  readonly source: string;
  readonly target: string | null;
  readonly side: DropSide;
  /** Set once it has been let go: the row keeps the drop's look until the layout follows. */
  readonly dropped: boolean;
}

/** What the row of tabs can do to a group before the layout changes it. */
export interface RowMotion {
  /** Draws the group's label and line back. */
  groupExit(group: GroupId): Promise<void>;
  /** Fades the group's label and tabs out. */
  groupFade(group: GroupId): Promise<void>;
}

export interface TabUiState {
  readonly renaming: TabId | null;
  /** The pane whose header shows a field for its name. */
  readonly renamingPane: PaneId | null;
  readonly popup: TabPopup | null;
  readonly drag: TabDrag | null;
  /** A tab just put in a group of its own: that group's editor opens once it is shown. */
  readonly editorFor: TabId | null;
  /** The row of tabs on show, while there is one. */
  readonly row: RowMotion | null;

  startRename(tab: TabId | null): void;
  endRename(): void;
  startPaneRename(pane: PaneId | null): void;
  endPaneRename(): void;
  open(popup: TabPopup): void;
  close(): void;
  setDrag(drag: TabDrag | null): void;
  awaitEditor(tab: TabId | null): void;
  setRow(row: RowMotion | null): void;
}

export type TabUiStore = StoreApi<TabUiState>;

/** The prefix that names a group's label among the tabs. */
export const LABEL_PREFIX = "L";

/** How the row names a group's label. */
export function labelId(group: GroupId): string {
  return `${LABEL_PREFIX}${group}`;
}

/** The group a name of the row's is the label of, if it is one. */
export function groupOfLabel(id: string): GroupId | null {
  return id.startsWith(LABEL_PREFIX) ? id.slice(LABEL_PREFIX.length) : null;
}

export function createTabUiStore(): TabUiStore {
  return createStore<TabUiState>()((set, get) => ({
    renaming: null,
    renamingPane: null,
    popup: null,
    drag: null,
    editorFor: null,
    row: null,
    startRename: (tab) => set({ renaming: tab, popup: null }),
    endRename: () => {
      if (get().renaming !== null) set({ renaming: null });
    },
    startPaneRename: (pane) => set({ renamingPane: pane, popup: null }),
    endPaneRename: () => {
      if (get().renamingPane !== null) set({ renamingPane: null });
    },
    open: (popup) => set({ popup }),
    close: () => {
      if (get().popup !== null) set({ popup: null });
    },
    setDrag: (drag) => set({ drag }),
    awaitEditor: (tab) => set({ editorFor: tab }),
    setRow: (row) => set({ row }),
  }));
}
