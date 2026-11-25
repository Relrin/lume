/**
 * The parts a window is made of, each kept apart from the others: one that cannot be drawn
 * says so in its place, and the terminals, the keyboard and everything else carry on.
 *
 * A part that failed is drawn again when the user asks, and by itself once something it
 * may have been drawn from has changed: what could not be drawn is most often something
 * that was heard from elsewhere, and is put right the same way.
 */

import { useEffect, useLayoutEffect, type ReactNode } from "react";

import { ErrorBoundary, type Failure } from "../ErrorBoundary.tsx";
import { PART_RETRY_MAX_MS, PART_RETRY_MIN_MS } from "../tuning.ts";
import type { ShellApp } from "./boot.ts";
import { useShell } from "./context.ts";

/** Calls `told` when something is heard that parts are drawn from. Returns how to stop. */
function followSources(app: ShellApp, told: () => void): () => void {
  const stops = [
    app.layout.subscribe(told),
    app.rex.subscribe(told),
    app.config.subscribe(told),
    app.profiles.subscribe(told),
    app.plugins.subscribe(told),
    app.status.subscribe(told),
    app.commandStore.subscribe(told),
  ];
  return () => stops.forEach((stop) => stop());
}

/** Draws a part again when something has changed, the more seldom the more it has failed. */
function useRetry({ again, times }: Failure): void {
  const app = useShell();
  // Before the page is drawn without the part: a change right after it failed counts.
  useLayoutEffect(() => {
    const wait = Math.min(PART_RETRY_MIN_MS * 2 ** Math.max(0, times - 1), PART_RETRY_MAX_MS);
    const from = performance.now() + wait;
    let timer: number | undefined;
    const stop = followSources(app, () => {
      timer ??= window.setTimeout(again, Math.max(0, from - performance.now()));
    });
    return () => {
      stop();
      window.clearTimeout(timer);
    };
  }, [app, again, times]);
}

/** What stands in the place of a part that failed. */
export function FailedPart({ what, failure }: { what: string; failure: Failure }) {
  useRetry(failure);
  return (
    <span className="part-failed" role="alert" title={failure.message}>
      <span className="part-failed-text">{what} could not be drawn.</span>
      <button type="button" className="part-failed-again" onClick={failure.again}>
        Try again
      </button>
    </span>
  );
}

/** Stands in the place of a part that is missed by nobody while it is not there. */
function Missing({ failure }: { failure: Failure }) {
  useRetry(failure);
  return null;
}

/** A part of the window. `what` is what it is called in "… could not be drawn". */
export function Part({
  what,
  quiet = false,
  children,
}: {
  what: string;
  /** Whether nothing is said where it fails: it is only there to be looked at. */
  quiet?: boolean;
  children: ReactNode;
}) {
  return (
    <ErrorBoundary
      fallback={(failure) =>
        quiet ? <Missing failure={failure} /> : <FailedPart what={what} failure={failure} />
      }
    >
      {children}
    </ErrorBoundary>
  );
}

/** Waits for something to be asked for again. */
function Waiting({ opens, again }: { opens: Opens | undefined; again: () => void }) {
  // After the page was told of the failure: putting away what failed is not asking for it.
  useEffect(() => opens?.(again), [opens, again]);
  return null;
}

/** Calls a listener when something that opens over the window is opened or put away. */
export type Opens = (listener: () => void) => () => void;

/**
 * Something that opens over the window: a palette, a dialog, a menu. One that cannot be
 * drawn is put away, which gives the keyboard back, and is said to have failed. `what` is
 * what it is called in "… could not be shown".
 */
export function Overlay({
  what,
  close,
  opens,
  children,
}: {
  what: string;
  /** Puts it away and gives the keyboard back. */
  close: () => void;
  /** How it is known to be asked for again. Left out where it goes with being put away. */
  opens?: Opens;
  children: ReactNode;
}) {
  const app = useShell();
  return (
    <ErrorBoundary
      fallback={({ again }) => <Waiting opens={opens} again={again} />}
      failed={() => {
        close();
        app.notices.push("error", `${what} could not be shown.`);
      }}
    >
      {children}
    </ErrorBoundary>
  );
}

/**
 * The questions put to the user. One that cannot be drawn is answered no, as a question
 * nobody saw must be, and the next is asked.
 */
export function Questions({ children }: { children: ReactNode }) {
  const app = useShell();
  return (
    <ErrorBoundary
      fallback={({ again }) => <Waiting opens={app.prompts.subscribe} again={again} />}
      failed={({ again }) => {
        app.prompts.current()?.answer(false);
        app.notices.push("error", "A question could not be shown, and was answered no.");
        // The next waits already, and nothing says so again.
        if (app.prompts.current()) queueMicrotask(again);
      }}
    >
      {children}
    </ErrorBoundary>
  );
}
