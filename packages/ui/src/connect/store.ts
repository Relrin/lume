/**
 * Whether the palette that opens a terminal from a profile is on show.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

/** The Connect palette's state. */
export interface ConnectState {
  /** Whether the palette is on show. */
  readonly shown: boolean;
  /** Counts the times it was opened: each opening begins afresh. */
  readonly opening: number;
  /** Shows the palette. */
  open(): void;
  /** Puts the palette away. */
  close(): void;
  /** Shows the palette, or puts it away if it is on show. */
  toggle(): void;
}

/** The store a window keeps its Connect palette's state in. */
export type ConnectStore = StoreApi<ConnectState>;

/** Makes a window's Connect store. The palette is put away to begin with. */
export function createConnectStore(): ConnectStore {
  return createStore<ConnectState>()((set, get) => ({
    shown: false,
    opening: 0,
    open: () => set({ shown: true, opening: get().opening + 1 }),
    close: () => {
      if (get().shown) set({ shown: false });
    },
    toggle: () => (get().shown ? get().close() : get().open()),
  }));
}
