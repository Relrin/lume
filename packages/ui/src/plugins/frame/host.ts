/**
 * What a plugin's frame needs of the window it is shown in: the way to the core, the
 * configuration the look comes from, and somewhere for the keys the frame passes on.
 * The terminal window is one; the settings window, which shows a plugin's settings
 * view on its page, gives a smaller one of its own.
 */

import { createContext, useContext } from "react";

import type { ConfigStore } from "../../config/store.ts";
import type { CoreClient } from "../../core/client.ts";
import type { ForwardedKey } from "../../keys/forwarded.ts";
import type { PaneId } from "../../layout/types.ts";
import { ShellContext } from "../../shell/context.ts";

export interface FrameHost {
  readonly client: CoreClient;
  readonly config: ConfigStore;
  /** Takes a key a frame did not use. A window with nothing to do with it has none. */
  forwardKey?(key: ForwardedKey, pane?: PaneId): void;
}

/** The window's own host for frames, where it is not a shell. */
export const FrameHostContext = createContext<FrameHost | null>(null);

/** What frames are given here: the window's own host, or else the shell. */
export function useFrameHost(): FrameHost {
  const own = useContext(FrameHostContext);
  const shell = useContext(ShellContext);
  const host = own ?? shell;
  if (!host) throw new Error("a plugin's frame is rendered where nothing hosts it");
  return host;
}
