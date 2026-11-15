/**
 * Keyboard events for the tests, built the way a browser fills them in.
 */

import type { KeyEventLike } from "../src/terminal/keyboard.ts";

/** What can be said about a key event beyond its key and code. */
export interface KeyOptions {
  ctrl?: boolean;
  shift?: boolean;
  alt?: boolean;
  meta?: boolean;
  repeat?: boolean;
  composing?: boolean;
  keyCode?: number;
  /** Lock and layout states, as `getModifierState` names them. */
  states?: readonly string[];
}

/** A key event. `key` is what the key typed or its name; `code` is the physical key. */
export function keyEvent(key: string, code: string, options: KeyOptions = {}): KeyEventLike {
  const states = new Set(options.states ?? []);
  return {
    key,
    code,
    ctrlKey: options.ctrl ?? false,
    shiftKey: options.shift ?? false,
    altKey: options.alt ?? false,
    metaKey: options.meta ?? false,
    repeat: options.repeat ?? false,
    isComposing: options.composing ?? false,
    keyCode: options.keyCode ?? 0,
    getModifierState: (state) => states.has(state),
  };
}
