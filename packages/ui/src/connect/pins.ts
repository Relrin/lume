/**
 * The profiles the user pinned in the connect palette, which it lists first.
 *
 * Pins are kept in the page's own storage, by profile id, between runs of the
 * application. A page that has no storage keeps them only while it is open.
 */

import { pageStorage, type RecentStorage } from "../palette/store.ts";

/** The name the pins are kept under. */
const PINS_KEY = "lume.connect.pins";

/** The most profiles kept pinned: more than any list needs at its top. */
export const PINS_MAX = 32;

/** The pinned profiles' ids, in the order they were pinned. */
export function readPins(storage: RecentStorage | undefined = pageStorage()): string[] {
  try {
    const kept: unknown = JSON.parse(storage?.getItem(PINS_KEY) ?? "[]");
    if (!Array.isArray(kept)) return [];
    return kept.filter((id): id is string => typeof id === "string").slice(0, PINS_MAX);
  } catch {
    // Storage that cannot be read, or holds something else, has no pins.
    return [];
  }
}

/** Pins a profile, or takes its pin away. Returns the pins as they now stand. */
export function togglePin(
  id: string,
  storage: RecentStorage | undefined = pageStorage(),
): string[] {
  const pins = readPins(storage);
  const next = pins.includes(id)
    ? pins.filter((pinned) => pinned !== id)
    : [...pins, id].slice(-PINS_MAX);
  try {
    storage?.setItem(PINS_KEY, JSON.stringify(next));
  } catch {
    // Storage that is full or forbidden: the pin lasts while the page is open.
  }
  return next;
}
