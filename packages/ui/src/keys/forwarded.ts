/**
 * Keys a plugin's view passes on.
 *
 * While the keyboard is in a plugin's frame, the window sees none of its keys: the
 * frame is another document. A view may pass on a key it has no use for, so that the
 * keys that move about the window still work from inside it. What it passes on is only
 * a description of a key, and a view can describe any key it likes, pressed or not.
 * So such a key is looked up in the bindings like any other, and then runs its command
 * only if the command is one of a fixed few that move the keyboard or show something
 * of the window's: another pane, another tab, the palette, a side panel, the bar.
 * Nothing that types, pastes, copies, closes, starts a program or belongs to a plugin
 * is among them, whatever the user has bound.
 */

import type { KeyEventLike } from "../terminal/keyboard.ts";

/** The commands a key passed on by a view may run. */
const ALLOWED: ReadonlySet<string> = new Set([
  "lume.pane.focus-left",
  "lume.pane.focus-right",
  "lume.pane.focus-up",
  "lume.pane.focus-down",
  "lume.pane.focus-next",
  "lume.pane.focus-previous",
  "lume.pane.focus-last",
  "lume.tab.next",
  "lume.tab.previous",
  "lume.tab.activate",
  "lume.palette.open",
  "lume.panel.toggle",
  "lume.window.focus-bar",
]);

/** Whether a key a view passed on may run `command`. */
export function mayForward(command: string): boolean {
  return ALLOWED.has(command);
}

/** A key as a view describes it. */
export interface ForwardedKey {
  readonly key: string;
  readonly code: string;
  readonly mods: readonly string[];
}

/**
 * Whether a view has any business passing a key on: one pressed with Ctrl, Alt or the
 * logo key, or a function key. Plain typing stays in the view.
 */
export function worthForwarding(key: ForwardedKey): boolean {
  if (key.mods.includes("ctrl") || key.mods.includes("alt") || key.mods.includes("meta")) {
    return true;
  }
  return /^F([1-9]|1[0-9]|2[0-4])$/.test(key.key);
}

/** A key a view passed on, as the bindings read a key that was pressed. */
export function forwardedEvent(key: ForwardedKey): KeyEventLike {
  const held = new Set(key.mods);
  return {
    key: key.key,
    code: key.code,
    ctrlKey: held.has("ctrl"),
    shiftKey: held.has("shift"),
    altKey: held.has("alt"),
    metaKey: held.has("meta"),
    // A view cannot hold a key down for the window, nor compose one.
    repeat: false,
    isComposing: false,
    keyCode: 0,
    getModifierState: () => false,
  };
}
