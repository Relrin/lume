/**
 * Whether the command palette is on show, and what it was opened for.
 *
 * The palette is opened by a key or a command to choose among every command, or by
 * something in the window that needs one command's arguments asked for. Which commands
 * were run from it lately is remembered between runs of the application, in the
 * page's own storage; a page that has no storage simply remembers nothing.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import { PALETTE_RECENTS_MAX } from "../tuning.ts";

/** The palette's state. */
export interface PaletteState {
  /** Whether the palette is on show. */
  readonly shown: boolean;
  /** The command whose arguments it asks for at once, when it was opened for one. */
  readonly command: string | null;
  /** Counts the times it was opened: each opening begins afresh. */
  readonly opening: number;
  /** Shows the palette with every command to choose from. */
  open(): void;
  /** Shows the palette asking for the arguments of one command. */
  openFor(command: string): void;
  /** Puts the palette away. */
  close(): void;
  /** Shows the palette, or puts it away if it is on show. */
  toggle(): void;
}

/** The store a window keeps its palette's state in. */
export type PaletteStore = StoreApi<PaletteState>;

/** Makes a window's palette store. The palette is put away to begin with. */
export function createPaletteStore(): PaletteStore {
  return createStore<PaletteState>()((set, get) => ({
    shown: false,
    command: null,
    opening: 0,
    open: () => set({ shown: true, command: null, opening: get().opening + 1 }),
    openFor: (command) => set({ shown: true, command, opening: get().opening + 1 }),
    close: () => {
      if (get().shown) set({ shown: false, command: null });
    },
    toggle: () => (get().shown ? get().close() : get().open()),
  }));
}

/** The part of the page's storage the palette uses. */
export type RecentStorage = Pick<Storage, "getItem" | "setItem">;

/** The name the palette's recent commands are kept under. */
const RECENTS_KEY = "lume.palette.recent";

/** The page's own storage, where the webview lets a page have one. */
export function pageStorage(): RecentStorage | undefined {
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

/** The ids of the commands last run from the palette, most recent first. */
export function readRecents(storage: RecentStorage | undefined): string[] {
  try {
    const kept: unknown = JSON.parse(storage?.getItem(RECENTS_KEY) ?? "[]");
    if (!Array.isArray(kept)) return [];
    return kept.filter((id): id is string => typeof id === "string").slice(0, PALETTE_RECENTS_MAX);
  } catch {
    // Storage that cannot be read, or holds something else, remembers nothing.
    return [];
  }
}

/** Records that a command was run from the palette. Returns the ids as they now stand. */
export function rememberRecent(storage: RecentStorage | undefined, command: string): string[] {
  const recents = [command, ...readRecents(storage).filter((id) => id !== command)].slice(
    0,
    PALETTE_RECENTS_MAX,
  );
  try {
    storage?.setItem(RECENTS_KEY, JSON.stringify(recents));
  } catch {
    // Storage that is full or forbidden: the palette works without it.
  }
  return recents;
}
