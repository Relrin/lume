/**
 * The window's key bindings: the ones keys pressed in the window are read by, and the
 * ones read for keys the plugins' views pass on.
 */

import type { Commands } from "../../commands/run.ts";
import type { ConnectStore } from "../../connect/store.ts";
import type { KeyBinding } from "../../core/types.ts";
import {
  forwardedEvent,
  mayForward,
  worthForwarding,
  type ForwardedKey,
} from "../../keys/forwarded.ts";
import { Shortcuts } from "../../keys/shortcuts.ts";
import type { LayoutStore } from "../../layout/store.ts";
import type { PaneId } from "../../layout/types.ts";
import type { PaletteStore } from "../../palette/store.ts";
import type { PluginsStore } from "../../plugins/store.ts";
import { currentPlatform, type KeyLayout, type Platform } from "../../terminal/keyboard.ts";
import type { Late } from "./late.ts";

/** No context holds besides the ones a key's place gives it. */
export const NO_CONTEXT: ReadonlySet<string> = new Set();

/** The context of a key pressed while the command palette is on show. */
const PALETTE_CONTEXT: ReadonlySet<string> = new Set(["palette"]);

/** The context of a key pressed while the Connect palette is on show. */
const CONNECT_CONTEXT: ReadonlySet<string> = new Set(["connect"]);

export interface InputDeps {
  readonly layout: LayoutStore;
  readonly palette: PaletteStore;
  readonly connect: ConnectStore;
  readonly plugins: PluginsStore;
  readonly keyLayout: KeyLayout;
  readonly bindings: readonly KeyBinding[];
  /** Running a command needs the views, and the views need the shortcuts. */
  readonly commands: Late<Commands>;
}

export interface Input {
  readonly platform: Platform;
  /** The bindings of keys pressed in the window. */
  readonly shortcuts: Shortcuts;
  /** The bindings of keys the plugins' views pass on. */
  readonly forwarded: Shortcuts;
  /** The context of the palette on show, if one is: it takes every key. */
  overlayContext(): ReadonlySet<string> | undefined;
  /** The contexts that hold where a key was pressed, with those the plugins set. */
  withPlugins(contexts: ReadonlySet<string>): ReadonlySet<string>;
  /** Takes a key one of the plugins' views passed on, from the view of `pane` if any. */
  forwardKey(key: ForwardedKey, pane?: PaneId): void;
  /** The keymap changed. */
  setBindings(bindings: readonly KeyBinding[]): void;
}

export function createInput(deps: InputDeps): Input {
  const { layout, palette, connect, plugins, keyLayout, commands } = deps;
  const overlayContext = (): ReadonlySet<string> | undefined => {
    if (palette.getState().shown) return PALETTE_CONTEXT;
    if (connect.getState().shown) return CONNECT_CONTEXT;
    return undefined;
  };
  const withPlugins = (contexts: ReadonlySet<string>): ReadonlySet<string> => {
    const set = plugins.getState().contexts;
    return set.size === 0 ? contexts : new Set([...contexts, ...set]);
  };

  const platform = currentPlatform();
  const shortcuts = new Shortcuts({ platform, layout: keyLayout }, (command, args) =>
    commands.get().run(command, args),
  );
  // Keys the plugins' views pass on are read by the same bindings, apart from the
  // window's own keys: a sequence begun in a view is not finished in a terminal. Of
  // what they are bound to, only the commands that move about the window are run.
  const forwarded = new Shortcuts({ platform, layout: keyLayout }, (command, args) => {
    if (mayForward(command)) commands.get().run(command, args);
  });
  const setBindings = (bindings: readonly KeyBinding[]): void => {
    shortcuts.setBindings(bindings);
    forwarded.setBindings(bindings);
  };
  setBindings(deps.bindings);

  const forwardKey = (key: ForwardedKey, pane?: PaneId): void => {
    if (!worthForwarding(key)) return;
    // While a palette is on show every key is the palette's own.
    if (overlayContext()) return;
    const contexts =
      pane !== undefined && layout.getState().floats[pane] ? new Set(["floating"]) : NO_CONTEXT;
    forwarded.handle(forwardedEvent(key), contexts);
  };

  return {
    platform,
    shortcuts,
    forwarded,
    overlayContext,
    withPlugins,
    forwardKey,
    setBindings,
  };
}
