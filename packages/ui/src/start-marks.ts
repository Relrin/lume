import { invoke } from "@tauri-apps/api/core";

/**
 * How far this page's start has come: the time each step was first reached, on the
 * page's own clock. Once the first output of a session is drawn the steps go to the core,
 * which puts them on the application's clock beside its own.
 */
export type StartStep = "parsed" | "vt" | "gpu" | "fonts" | "layout" | "frame" | "output" | "drawn";

const reached: Partial<Record<StartStep, number>> = {};
let output = false;
let told = false;

/** Notes that a step of the start was reached; only the first time counts. */
export function startStep(step: StartStep, at: number = performance.now()): void {
  reached[step] ??= at;
}

/** Notes that output of a session reached the page. Costs one check once noted. */
export function startOutput(): void {
  if (output) return;
  output = true;
  startStep("output");
}

/**
 * Notes that a frame was drawn. The first one drawn after output arrived is the end of
 * the start, and the steps are told to the core then, once.
 */
export function startDrawn(): void {
  if (told) return;
  startStep("frame");
  if (!output) return;
  told = true;
  startStep("drawn");
  tellCore();
}

/**
 * Tells the core when the first frame after the window was shown again is drawn: the
 * end of a launch of an application that was kept ready.
 */
export function shownAgain(): void {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
  requestAnimationFrame(() => {
    invoke("page_shown").catch(() => undefined);
  });
}

/** The steps reached so far, for tests. */
export function startSteps(): Readonly<Partial<Record<StartStep, number>>> {
  return reached;
}

function tellCore(): void {
  // Only inside the application is there a core to tell.
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
  invoke("page_marks", { steps: { ...reached }, now: performance.now() }).catch(() => undefined);
}

startStep("parsed");
