/**
 * What a scene of the window's page is: an arrangement of the window to write down and
 * compare with the one last found right.
 */

import type { ShellApp } from "../shell/boot.ts";
import type { FakeCore, FakeCoreOptions } from "./fake-core.ts";

/** One arrangement of the window to write down. */
export interface Scene {
  /** Names the scene, and its reference file. */
  readonly name: string;
  readonly window: FakeCoreOptions;
  /**
   * What is done to the page once it has started, before it is written down. The page
   * is written down once what this returns has settled.
   */
  readonly prepare?: (app: ShellApp, core: FakeCore) => void | Promise<void>;
  /** What must hold of the page once it has settled, beyond what is written down. */
  readonly check?: (page: HTMLElement) => string[];
}
