/**
 * The configuration in use, as the page last heard it.
 *
 * The core resolves the user's files to one view: the settings, the design tokens and
 * the keys. A page takes the view whole when it starts and again whenever the core
 * says it changed. Views are numbered, and one that is not newer than the one held is
 * dropped: an answer to a request may arrive after a notice that is newer than it.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { ConfigProblem, ConfigView } from "../core/types.ts";

/** The configuration a page holds. */
export interface ConfigState {
  /** The view in use. Nothing until the core has sent one. */
  readonly view: ConfigView | null;
  /**
   * Why the files, as they were last saved, could not be used: the view held is the one
   * from before. Empty once a view is taken again.
   */
  readonly failed: readonly ConfigProblem[];
  /** Takes a view, unless the one held is as new or newer. Says whether it was taken. */
  set(view: ConfigView): boolean;
  /** Notes that the files could not be used, and why. */
  fail(problems: readonly ConfigProblem[]): void;
}

/** The store a page keeps its configuration in. */
export type ConfigStore = StoreApi<ConfigState>;

/** Makes a page's configuration store. It holds nothing until it is given a view. */
export function createConfigStore(): ConfigStore {
  return createStore<ConfigState>()((set, get) => ({
    view: null,
    failed: [],
    set: (view) => {
      const held = get().view;
      if (held !== null && view.revision <= held.revision) return false;
      set({ view, failed: [] });
      return true;
    },
    fail: (problems) => set({ failed: problems }),
  }));
}
