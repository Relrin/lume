/**
 * The status items a window shows, by the slot each is shown in.
 *
 * Whoever has something to show registers an item under an id and updates or removes
 * it by that id. Each slot's items are kept as a list in the order they are shown, and
 * a slot's list is replaced only when one of its own items changes, so a meter that
 * moves in one slot leaves whatever draws the other slots alone.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import { STATUS_SLOTS, type StatusItem, type StatusSlot } from "./types.ts";

/** No items: one list for every slot that has none. */
const NO_ITEMS: readonly StatusItem[] = [];

/** What of one kind of item may change once it is shown: all but its id and its kind. */
type ChangesTo<Item> = Item extends unknown ? Partial<Omit<Item, "id" | "kind">> : never;

/** What of an item may change once it is shown. */
export type StatusChanges = ChangesTo<StatusItem>;

/** The status items a window holds. */
export interface StatusState {
  /** Each slot's items in the order they are shown. */
  readonly bySlot: Readonly<Record<StatusSlot, readonly StatusItem[]>>;
  /** Shows an item, in place of any item of the same id. */
  register(item: StatusItem): void;
  /**
   * Changes an item that is shown. The item keeps its id and its kind; an id nothing
   * was registered under changes nothing. Says whether there was such an item.
   */
  update(id: string, changes: StatusChanges): boolean;
  /** Takes an item away. */
  remove(id: string): void;
  /** The item with this id, if one is shown. */
  find(id: string): StatusItem | undefined;
}

/** The store a window keeps its status items in. */
export type StatusStore = StoreApi<StatusState>;

function inOrder(a: StatusItem, b: StatusItem): number {
  const byOrder = (a.order ?? 0) - (b.order ?? 0);
  if (byOrder !== 0) return byOrder;
  return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
}

function emptySlots(): Record<StatusSlot, readonly StatusItem[]> {
  return {
    "topbar.left": NO_ITEMS,
    "topbar.center": NO_ITEMS,
    "topbar.right": NO_ITEMS,
    "statusbar.left": NO_ITEMS,
    "statusbar.right": NO_ITEMS,
    "pane.header": NO_ITEMS,
  };
}

/** Makes a window's status registry. It shows nothing until something registers. */
export function createStatusStore(): StatusStore {
  return createStore<StatusState>()((set, get) => {
    /** The slots with `id` taken out of whichever holds it. */
    const without = (id: string) => {
      const next = { ...get().bySlot };
      for (const slot of STATUS_SLOTS) {
        if (!next[slot].some((item) => item.id === id)) continue;
        const left = next[slot].filter((item) => item.id !== id);
        next[slot] = left.length === 0 ? NO_ITEMS : left;
      }
      return next;
    };
    const put = (item: StatusItem) => {
      const next = without(item.id);
      next[item.slot] = [...next[item.slot], item].sort(inOrder);
      set({ bySlot: next });
    };
    const find = (id: string) => {
      for (const slot of STATUS_SLOTS) {
        const found = get().bySlot[slot].find((item) => item.id === id);
        if (found) return found;
      }
      return undefined;
    };
    return {
      bySlot: emptySlots(),
      register: put,
      update: (id, changes) => {
        const held = find(id);
        if (!held) return false;
        // The kind and the id are the item's own; everything else may change.
        put({ ...held, ...changes, id: held.id, kind: held.kind } as StatusItem);
        return true;
      },
      remove: (id) => {
        if (find(id)) set({ bySlot: without(id) });
      },
      find,
    };
  });
}
