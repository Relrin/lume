/**
 * The Rex servers there are and how each stands, as the page last heard them.
 *
 * The core keeps the connections and says when anything about them changes, always as
 * the whole list. The page asks for the list once when it starts, without waiting for
 * it, and takes whatever is newer from then on: an answer may arrive after a notice
 * that is newer than it.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { RexHosts } from "../core/types.ts";

/** The servers a page holds. */
export interface RexState {
  /** The list in use. Nothing until the core has sent one. */
  readonly list: RexHosts | null;
  /** Takes a list, unless the one held is as new or newer. Says whether it was taken. */
  set(list: RexHosts): boolean;
}

/** The store a page keeps the servers in. */
export type RexStore = StoreApi<RexState>;

/** Makes a page's store of servers. It holds nothing until it is given a list. */
export function createRexStore(): RexStore {
  return createStore<RexState>()((set, get) => ({
    list: null,
    set: (list) => {
      const held = get().list;
      if (held !== null && list.revision <= held.revision) return false;
      set({ list });
      return true;
    },
  }));
}
