/**
 * What a window shows when nothing of it can be drawn: that, why, and the two ways on.
 *
 * Whatever runs in the window runs on. Drawing again keeps the page as it is; reloading
 * begins the page afresh and shows the same sessions.
 */

import { useEffect, useRef, type ReactNode } from "react";
import { useStore } from "zustand";

import { ErrorBoundary, type Failure } from "../ErrorBoundary.tsx";
import { useShell } from "./context.ts";
import { WindowControls } from "./WindowControls.tsx";

/** The whole window in words. `bar` is the window's own bar, where the page draws it. */
export function WindowFailed({
  title,
  failure,
  bar,
}: {
  title: string;
  failure: Failure;
  bar?: ReactNode;
}) {
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => first.current?.focus(), []);
  return (
    <div className="shell window-failed">
      <ErrorBoundary fallback={() => null}>{bar}</ErrorBoundary>
      <main className="start-failure" role="alert">
        <h1>{title}</h1>
        <p>{failure.message}</p>
        <div className="window-failed-ways">
          <button ref={first} type="button" className="connect-primary" onClick={failure.again}>
            Draw again
          </button>
          <button
            type="button"
            className="connect-secondary"
            onClick={() => window.location.reload()}
          >
            Reload
          </button>
        </div>
      </main>
    </div>
  );
}

/** The bar of a terminal window of which nothing else can be drawn. */
export function BareBar() {
  const app = useShell();
  const custom = useStore(app.config, (state) => state.view?.title_bar === "custom");
  const mac = useStore(app.config, (state) => state.view?.platform === "macos");
  if (!custom) return null;
  return (
    <header
      className="top-bar"
      data-title-bar="custom"
      onMouseDown={(event) => {
        if (event.button !== 0 || (event.target as Element).closest("button")) return;
        event.preventDefault();
        const asked =
          event.detail === 2 ? app.client.windowControl("maximise") : app.client.windowDrag();
        asked.catch(() => undefined);
      }}
    >
      <div className="top-centre" />
      <div className="top-right">{!mac && <WindowControls />}</div>
    </header>
  );
}
