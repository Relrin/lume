/**
 * Whether the application is connected to the session host, as the core last said.
 *
 * Only the core knows. The page reads it with its layout when it starts and is told of
 * every change; the dot beside the workspace's name is drawn from what is held here.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

/** The connection to the session host, and the way to say what it is now. */
export interface HostLinkHolder {
  readonly connected: boolean;
  set(connected: boolean): void;
}

/** The store a page keeps the state of the connection in. */
export type HostLinkStore = StoreApi<HostLinkHolder>;

/** Makes a page's store: connected, until told otherwise. */
export function createHostLinkStore(): HostLinkStore {
  return createStore<HostLinkHolder>()((set, get) => ({
    connected: true,
    set: (connected) => {
      if (get().connected !== connected) set({ connected });
    },
  }));
}
