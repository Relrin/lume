/**
 * How the window's terminals are drawn: the renderers' host, on the tier the
 * configuration allows, and the look of the terminals with its fonts loaded.
 */

import { RenderHost, TIER_ORDER, type Tier } from "@lume/renderer";
import { terminalAppearance } from "@lume/tokens";

import { applyConfig } from "../../config/apply.ts";
import { loadTerminalFonts } from "../../config/fonts.ts";
import type { ConfigStore } from "../../config/store.ts";
import type { ConfigView } from "../../core/types.ts";
import type { TabAreaController } from "../../layout/controller.ts";
import type { PaneViews } from "../../panes/views.ts";
import { firstFrame, startStep } from "../../start-marks.ts";
import { DEFAULT_APPEARANCE, type Appearance } from "../../theme.ts";
import { GPU_ASK_LATEST_MS } from "../../tuning.ts";
import type { Cursor } from "./cursor.ts";
import type { Input } from "./input.ts";

/** The tiers the configuration lets a terminal be drawn with, best first. */
function tiersFor(view: ConfigView): readonly Tier[] {
  const tier = view.config.renderer.tier;
  return tier === "auto" ? TIER_ORDER : [tier];
}

/**
 * Starts the host before the configuration is known. Nothing waits for the graphics
 * device: terminals are drawn with the next tier until it is granted. It is asked for
 * only once a terminal's first frame is on screen, since the browser's GPU process,
 * which shows the page's frames, is held up for some hundreds of milliseconds by the
 * asking.
 */
export function startHost(): RenderHost {
  return RenderHost.start({
    gpuAfter: Promise.race([
      firstFrame.then(() => new Promise((shown) => requestAnimationFrame(shown))),
      new Promise((late) => setTimeout(late, GPU_ASK_LATEST_MS)),
    ]),
  });
}

/**
 * The host the window keeps: the one started, unless the configuration names one tier,
 * which is rare; then that one is made in its place.
 */
export async function settleHost(started: RenderHost, view: ConfigView): Promise<RenderHost> {
  let host = started;
  if (view.config.renderer.tier !== "auto") {
    started.dispose();
    host = await RenderHost.create({ tiers: tiersFor(view) });
  }
  void host.gpuSettled.then(() => startStep("gpu"));
  return host;
}

/** The look of the window's terminals, worked out once for each configuration. */
export class TerminalLook {
  readonly #config: ConfigStore;
  #current: Appearance;

  constructor(config: ConfigStore) {
    this.#config = config;
    this.#current = this.#work();
  }

  /** The look for the configuration in use. */
  get current(): Appearance {
    return this.#current;
  }

  /** Works the look out again, as for a configuration that changed; gives it back. */
  refresh(): Appearance {
    this.#current = this.#work();
    return this.#current;
  }

  /** The look as worked out from the configuration in use, and its fonts once loaded. */
  static async load(config: ConfigStore): Promise<TerminalLook> {
    const look = new TerminalLook(config);
    // A terminal sizes its cells and draws its glyphs once: the font is there first.
    await loadTerminalFonts(look.current);
    startStep("fonts");
    return look;
  }

  #work(): Appearance {
    const view = this.#config.getState().view;
    return view ? terminalAppearance(view.tokens, view.theme.scheme) : DEFAULT_APPEARANCE;
  }
}

export interface ConfigFollowDeps {
  readonly look: TerminalLook;
  readonly views: PaneViews;
  readonly area: TabAreaController;
  readonly cursor: Cursor;
  readonly input: Input;
}

/** What the window does when the configuration in use is another. */
export function followConfig({
  look,
  views,
  area,
  cursor,
  input,
}: ConfigFollowDeps): (view: ConfigView) => void {
  return (view: ConfigView): void => {
    const settled = look.refresh();
    applyConfig(view);
    cursor.configure();
    input.setBindings(view.keymap.bindings);
    // The terminals are set up again once the font the configuration now names is
    // ready, unless the configuration has changed again by then.
    void loadTerminalFonts(settled).then(() => {
      if (look.current !== settled) return;
      views.configure();
      area.layout();
    });
  };
}
