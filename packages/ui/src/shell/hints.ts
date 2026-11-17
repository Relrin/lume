/**
 * The keys that run a command, for the tooltip of the control that runs it too, and
 * for the end of a menu's item that runs it.
 */

import { useStore } from "zustand";

import { keyLabel } from "../commands/store.ts";
import type { ConfigView } from "../core/types.ts";
import { useShell } from "./context.ts";

/**
 * The keys bound to a command, written for a tooltip: ` (Ctrl+Shift+T)`, or nothing
 * when no key runs it. With `args`, only a key bound to the command with exactly those
 * arguments counts.
 */
export function useKeyHint(command: string, args?: unknown): string {
  const { config } = useShell();
  return useStore(config, (state) => {
    const view = state.view;
    const keys = view && keyLabel(view.keymap.bindings, command, view.platform, args);
    return keys ? ` (${keys})` : "";
  });
}

/**
 * A key as a menu writes it: the keys of a chord set apart by spaces, as in
 * `Ctrl Shift N`. Where chords are written without anything between their keys, as on
 * a Mac, there is nothing to change.
 */
export function menuKey(label: string | undefined): string | undefined {
  // A `+` after a key's name joins; one after another `+` is the key itself.
  return label?.replace(/(?<=[^+\s])\+(?=.)/g, " ");
}

/** The keys bound to a command as a menu writes them, or nothing when no key runs it. */
export function menuHint(view: ConfigView | null, command: string): string | undefined {
  return view ? menuKey(keyLabel(view.keymap.bindings, command, view.platform)) : undefined;
}
