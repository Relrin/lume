/**
 * What a scene of the window's page is: an arrangement of the window to write down and
 * compare with the one last found right.
 */

import type { ShellApp } from "../shell/boot/index.ts";
import type { FakeCore, FakeCoreOptions } from "./fake-core.ts";

/** One arrangement of the window to write down. */
export interface Scene {
  /** Names the scene, and its reference file. */
  readonly name: string;
  readonly window: FakeCoreOptions;
  /**
   * How wide the window is, in CSS pixels, where it is not the usual width: for a scene
   * whose top bar would otherwise be so full that whether its tabs fit depends on how
   * the engine measures text.
   */
  readonly width?: number;
  /**
   * What is done to the page once it has started, before it is written down. The page
   * is written down once what this returns has settled.
   */
  readonly prepare?: (app: ShellApp, core: FakeCore) => void | Promise<void>;
  /** What must hold of the page once it has settled, beyond what is written down. */
  readonly check?: (page: HTMLElement) => string[];
}
