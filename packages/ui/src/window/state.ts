/**
 * What the page's window is doing on the desktop, as the core last said: whether it is
 * maximised, fills the screen, or is the window the user is working in.
 *
 * Only the core knows. The page asks once when it starts and is told of every change,
 * and draws its own window buttons from what it holds here.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { WindowState } from "../core/types.ts";

/** The window's state, and the way to replace it. */
export interface WindowStateHolder extends WindowState {
  /** Takes what the core says the window is doing now. */
  set(state: WindowState): void;
}

/** The store a page keeps its window's state in. */
export type WindowStateStore = StoreApi<WindowStateHolder>;

/** Makes a page's window state store: an ordinary window in front, until told otherwise. */
export function createWindowStateStore(): WindowStateStore {
  return createStore<WindowStateHolder>()((set, get) => ({
    maximised: false,
    fullscreen: false,
    focused: true,
    set: ({ maximised, fullscreen, focused }) => {
      const held = get();
      const same =
        held.maximised === maximised && held.fullscreen === fullscreen && held.focused === focused;
      if (!same) set({ maximised, fullscreen, focused });
    },
  }));
}
