/**
 * Motion plugins bring for the window's places, played.
 *
 * The settings choose, place by place, a plugin's effect over the window's own: how
 * menus open, how notices come and go, how the palette, panels, floating panes and the
 * pills of panes put away come in, how a status item shows that it changed. The core
 * checked each effect before the page was sent it: a few properties, one iteration,
 * never more than a second, ending where the thing rests. Here an effect is handed to
 * the browser's own animations, which a page may run under its content policy.
 *
 * While the system asks for less motion nothing a plugin brings plays.
 */

import type { ConfigView, MotionDef, MotionEasing, UiContributions } from "../core/types.ts";
import { springCurve } from "../shell/tab-motion.ts";
import type { MotionChoices } from "../wire/config.generated.ts";

/** The places a plugin's effect plays in, by the name of their setting. */
export type MotionSlot =
  | "menu.enter"
  | "toast.enter"
  | "toast.exit"
  | "palette.enter"
  | "panel.enter"
  | "float.enter"
  | "pill.enter"
  | "status.change";

/** The setting of a place, under `appearance.motion`. */
function settingOf(slot: MotionSlot): keyof MotionChoices {
  return slot.replace(".", "-") as keyof MotionChoices;
}

/** The effect a plugin brings that the settings choose for a place, if one is chosen and there. */
export function chosenMotion(
  view: ConfigView | null,
  shown: UiContributions | null,
  slot: MotionSlot,
): MotionDef | undefined {
  const chosen = view?.config.appearance.motion?.[settingOf(slot)];
  if (chosen === undefined) return undefined;
  return shown?.motions.find((motion) => motion.id === chosen && motion.slot === slot);
}

/** The value of a design token's variable as the page has it now. */
function tokenValue(name: string): string {
  return getComputedStyle(document.documentElement)
    .getPropertyValue(`--lume-motion-easing-${name}`)
    .trim();
}

/** A curve as the browser's animations take it, and the length a spring gives it. */
export function easingOf(
  easing: MotionEasing,
  duration: number,
): { readonly easing: string; readonly duration: number } {
  if ("css" in easing) return { easing: easing.css, duration };
  if ("token" in easing) return { easing: tokenValue(easing.token) || "ease", duration };
  const curve = springCurve([easing.spring[0], easing.spring[1]]);
  return { easing: curve.easing, duration: curve.duration };
}

/** Whether the system asks for less motion. */
function lessMotion(): boolean {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/**
 * Plays a plugin's effect on an element. The element's own animation from the
 * stylesheet is set aside for it while it plays. Gives the animation, or nothing when
 * nothing plays.
 */
export function playMotion(element: HTMLElement, motion: MotionDef): Animation | undefined {
  const spec = motion.spec;
  if (!("keyframes" in spec) || lessMotion() || typeof element.animate !== "function") {
    return undefined;
  }
  const { easing, duration } = easingOf(spec.easing, spec.duration);
  element.dataset["motion"] = "plugin";
  const animation = element.animate(spec.keyframes as Keyframe[], {
    duration,
    delay: spec.delay,
    easing,
    iterations: 1,
    fill: "none",
  });
  const done = () => {
    if (element.dataset["motion"] === "plugin") delete element.dataset["motion"];
  };
  animation.addEventListener("finish", done);
  animation.addEventListener("cancel", done);
  return animation;
}
