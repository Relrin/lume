/**
 * The ways there are to start a terminal, as the page last heard them.
 *
 * The core finds profiles on the machine and reads the user's own; it says when the
 * list changes. The page asks for the list once when it starts, without waiting for it,
 * and takes whatever is newer from then on: an answer may arrive after a notice that is
 * newer than it.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { Profile, ProfileList } from "../core/types.ts";

/** The profiles a page holds. */
export interface ProfilesState {
  /** The list in use. Nothing until the core has sent one. */
  readonly list: ProfileList | null;
  /** Takes a list, unless the one held is as new or newer. Says whether it was taken. */
  set(list: ProfileList): boolean;
}

/** The store a page keeps the profiles in. */
export type ProfilesStore = StoreApi<ProfilesState>;

/** Makes a page's profile store. It holds nothing until it is given a list. */
export function createProfilesStore(): ProfilesStore {
  return createStore<ProfilesState>()((set, get) => ({
    list: null,
    set: (list) => {
      const held = get().list;
      if (held !== null && list.revision <= held.revision) return false;
      set({ list });
      return true;
    },
  }));
}

/** The profile called `id` in a list, if there is one. */
export function findProfile(
  list: ProfileList | null,
  id: string | null | undefined,
): Profile | undefined {
  if (!list || id === null || id === undefined) return undefined;
  return list.profiles.find((profile) => profile.id === id);
}
