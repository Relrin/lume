/**
 * The plugins' frames open in this window, by the name the page gave each, so what the
 * application sends a view finds it.
 */

import type { FrameBridge } from "./bridge.ts";

const open = new Map<string, FrameBridge>();
let next = 1;

/** A name for a view this page opens, which no other of its views has. */
export function nextViewName(): string {
  const name = `v${next}`;
  next += 1;
  return name;
}

/** Keeps a frame's bridge under its view's name; gives what takes it back out. */
export function keepFrame(view: string, bridge: FrameBridge): () => void {
  open.set(view, bridge);
  return () => {
    if (open.get(view) === bridge) open.delete(view);
  };
}

/** Passes an event from a plugin's code to one of its open views here. */
export function deliverViewEvent(view: string, event: string, payload: unknown): void {
  open.get(view)?.event(event, payload);
}

/** How to give each pane's frame the keyboard, for the panes whose frames are ready. */
const paneFrames = new Map<string, () => void>();

let frameReady: ((pane: string) => void) | null = null;

/** Sets who hears that a pane's frame is ready to be given the keyboard. */
export function onPaneFrameReady(listener: ((pane: string) => void) | null): void {
  frameReady = listener;
}

/**
 * Keeps the way to give a pane's frame the keyboard, from when the frame is ready
 * until it goes; gives what takes it back out.
 */
export function keepPaneFrame(pane: string, focus: () => void): () => void {
  paneFrames.set(pane, focus);
  frameReady?.(pane);
  return () => {
    if (paneFrames.get(pane) === focus) paneFrames.delete(pane);
  };
}

/** Gives the keyboard to the frame of a pane, if the pane has one that is ready. */
export function focusPaneFrame(pane: string): boolean {
  const focus = paneFrames.get(pane);
  if (!focus) return false;
  focus();
  return true;
}

/** Every open frame, for telling all of them something. */
export function openFrames(): readonly FrameBridge[] {
  return [...open.values()];
}
