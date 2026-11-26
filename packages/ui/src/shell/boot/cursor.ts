/**
 * The cursor's effects. They are drawn over the whole window, so that a trail can go
 * from one pane to another; the page gives them their element when it is rendered.
 */

import type { RenderHost } from "@lume/renderer";

import type { ConfigStore } from "../../config/store.ts";
import type { CoreClient } from "../../core/client.ts";
import { assets } from "../../plugins/assets.ts";
import type { PluginsStore } from "../../plugins/store.ts";
import { CursorFx, cursorFxSettings, cursorFxShaders } from "../../terminal/cursor-fx.ts";
import { sentence, type NoticeStore } from "../notices.ts";
import type { Disposer } from "./disposer.ts";

export interface CursorDeps {
  readonly host: RenderHost;
  readonly client: CoreClient;
  readonly config: ConfigStore;
  readonly plugins: PluginsStore;
  readonly notices: NoticeStore;
  readonly disposer: Disposer;
}

export interface Cursor {
  readonly cursorFx: CursorFx;
  /**
   * Sets the effects up for what the configuration names, with the plugins' shaders
   * among them: a shader not fetched yet is fetched, and the effects are set up again
   * when it is there, unless something else was chosen meanwhile.
   */
  configure(): void;
}

export function startCursor({
  host,
  client,
  config,
  plugins,
  notices,
  disposer,
}: CursorDeps): Cursor {
  const lessMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const cursorFx = new CursorFx((container) => host.createCursorEffects(container), lessMotion);
  disposer.add(() => cursorFx.detach());
  // The plugins' pictures and shaders are fetched as they are needed.
  disposer.add(assets.connect((hashes) => client.uiAssets(hashes)));

  const configure = (): void => {
    const view = config.getState().view;
    const shown = plugins.getState().shown;
    cursorFx.configure(cursorFxSettings(view, shown));
    const wanted = cursorFxShaders(view, shown).filter((hash) => assets.peek(hash) === undefined);
    if (wanted.length === 0) return;
    void assets.load(wanted).then(() => {
      if (wanted.some((hash) => assets.peek(hash) !== undefined)) {
        cursorFx.configure(cursorFxSettings(config.getState().view, plugins.getState().shown));
      }
    });
  };
  configure();
  disposer.add(
    cursorFx.onProblem((id, message) => {
      const named = plugins.getState().shown?.cursor_effects.find((effect) => effect.id === id);
      notices.push(
        "warning",
        `The cursor effect ${named?.title ?? id} cannot be drawn, so none is: ${sentence(message)}`,
      );
    }),
  );

  const onLessMotion = () => cursorFx.configure();
  lessMotion.addEventListener("change", onLessMotion);
  disposer.add(() => lessMotion.removeEventListener("change", onLessMotion));
  const size = () =>
    cursorFx.resize(
      { width: window.innerWidth, height: window.innerHeight },
      window.devicePixelRatio,
    );
  size();
  // A move to a display of another pixel ratio resizes the window's page too.
  window.addEventListener("resize", size);
  disposer.add(() => window.removeEventListener("resize", size));

  return { cursorFx, configure };
}
