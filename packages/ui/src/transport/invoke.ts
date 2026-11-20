/**
 * Every call the page makes to the application goes through here, with the page's token.
 *
 * The application gives each page it loads a secret, once: the first request for it
 * after the page starts loading gets it, and later ones are refused. Plugins' frames live
 * in the same webview, so the secret is kept in this module's closure — never on
 * `window`, in the DOM or in storage — and every command carries it in a header. A
 * command without it is refused before it runs.
 *
 * The page asks for the token before it makes any frame; `ready()` is that first step of
 * every page's boot, and calls made earlier wait for it.
 */

import { invoke as tauriInvoke, type InvokeArgs, type InvokeOptions } from "@tauri-apps/api/core";

/** The header the token travels in. */
export const TOKEN_HEADER = "lume-frame";

let token: Promise<string> | undefined;

/** Fetches the page's token, once; every later call shares the same answer. */
export function ready(): Promise<string> {
  token ??= tauriInvoke<string>("frame_token");
  return token;
}

/** Calls an application command with the page's token. */
export async function invoke<T>(
  command: string,
  args?: InvokeArgs,
  options?: InvokeOptions,
): Promise<T> {
  const secret = await ready();
  const headers = new Headers(options?.headers);
  headers.set(TOKEN_HEADER, secret);
  return tauriInvoke<T>(command, args, { ...options, headers });
}
