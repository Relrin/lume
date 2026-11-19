/**
 * The window's places of motion, as its components play them.
 *
 * A component that comes in at one of the places calls the hook for it with its
 * element; the effect a plugin brings for the place plays if the settings choose one,
 * and the element's own animation from the stylesheet plays otherwise. The window's
 * page says, once, where the settings and the plugins' effects are read from; a page
 * that says nothing, as the settings window's does not, plays only its own.
 */

import { useLayoutEffect, type RefObject } from "react";

import type { ConfigView, UiContributions } from "../core/types.ts";
import { chosenMotion, playMotion, type MotionSlot } from "./play.ts";

/** Where the settings and the plugins' effects are read from. */
export interface MotionSource {
  view(): ConfigView | null;
  shown(): UiContributions | null;
}

let source: MotionSource | null = null;

/** Says where the settings and the plugins' effects are read from. */
export function setMotionSource(next: MotionSource | null): void {
  source = next;
}

/** Plays the effect chosen for a place on an element now. Gives the animation, if one plays. */
export function playSlot(slot: MotionSlot, element: HTMLElement | null): Animation | undefined {
  if (!element || !source) return undefined;
  const motion = chosenMotion(source.view(), source.shown(), slot);
  return motion ? playMotion(element, motion) : undefined;
}

/**
 * Plays the effect chosen for a place on the element when it comes in, and again
 * whenever `again` changes.
 */
export function useSlotMotion(
  slot: MotionSlot,
  element: RefObject<HTMLElement | null>,
  again?: unknown,
): void {
  useLayoutEffect(() => {
    const animation = playSlot(slot, element.current);
    return () => animation?.cancel();
    // The element is the same one for as long as the component is.
  }, [slot, again]);
}

/**
 * Plays the effect chosen for a place on an element that goes away, and resolves once
 * it is over, at once when none is chosen.
 */
export function leaveSlot(slot: MotionSlot, element: HTMLElement | null): Promise<void> {
  const animation = playSlot(slot, element);
  if (!animation) return Promise.resolve();
  return animation.finished.then(
    () => undefined,
    () => undefined,
  );
}
