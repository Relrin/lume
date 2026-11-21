import { loadTerminalFonts } from "../config/fonts.ts";
import { webviewInfo } from "../probe.ts";
import { DEFAULT_APPEARANCE } from "../theme.ts";
import { ready } from "../transport/invoke.ts";
import { shellApp, startVt } from "./boot.ts";

/**
 * Begins, as the page's script starts, what a window's start otherwise reaches one
 * step after another: the page's token, what the core says of the webview, the terminal
 * emulator, the terminal's own font, and, once the core has said this window shows
 * terminals, the terminals themselves. Each is asked for once, so whoever asks later
 * is given the same answer, usually one that is there already.
 *
 * Nothing here is needed for the page to work: a step that was not begun here is begun
 * where it is first wanted.
 */
export function startEarly(): void {
  // Only inside the application is there a core to ask.
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
  const nothing = (): undefined => undefined;
  ready().catch(nothing);
  startVt().catch(nothing);
  // The font most terminals are drawn in. One that asks for another loads its own.
  loadTerminalFonts(DEFAULT_APPEARANCE).catch(nothing);
  webviewInfo().then((info) => {
    // A window that reports on the webview, or runs a test suite, has no terminals.
    if (info && !info.probe && !info.harness) shellApp().catch(nothing);
  }, nothing);
}
