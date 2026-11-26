/** What the page loads once, whatever else it does: the terminal engine and the keyboard. */

import { Vt } from "@lume/vt";
import wasmUrl from "@lume/vt/ghostty-vt.wasm?url";

import { KeyLayout } from "../../terminal/keyboard.ts";

let emulator: Promise<Vt> | undefined;

/**
 * The terminal emulator, fetched and compiled once for the page. A starting window asks
 * for it before anything else, so that it is there when the first terminal is made.
 */
export function startVt(): Promise<Vt> {
  emulator ??= Vt.instantiate(fetch(wasmUrl));
  return emulator;
}

/** The parts of the keyboard-layout interface some webviews offer. */
interface KeyboardLike {
  getLayoutMap(): Promise<Iterable<[string, string]>>;
}

/** The keyboard layout as the webview reports it, where it can. */
export async function keyboardLayout(): Promise<KeyLayout> {
  const keyboard = (navigator as Navigator & { keyboard?: KeyboardLike }).keyboard;
  try {
    if (keyboard) return new KeyLayout(new Map(await keyboard.getLayoutMap()));
  } catch {
    // Not allowed or not supported: the layout is learnt from what is typed instead.
  }
  return new KeyLayout();
}
