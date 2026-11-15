/**
 * The keys that run a command, for the tooltip of the control that runs it too.
 */

import { useStore } from "zustand";

import { keyLabel } from "../commands/store.ts";
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
