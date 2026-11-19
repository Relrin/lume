/**
 * How the mark of the tab on show moves, as the settings choose it: one of the
 * window's own styles, or a plugin's, written as the window's own are.
 *
 * A plugin's style is the same data as the window's: the shape of the mark, the
 * springs its two ends move with, how tabs slide and come in. A style whose plugin is
 * not there is the window's glow.
 */

import type { Contributed, MotionDef, TabHighlight } from "../core/types.ts";
import { MOTION_STYLES, type MotionStyle, type Spring } from "../shell/tab-motion.ts";
import { easingOf } from "./play.ts";

/** The styles of the window's own, by the setting that chooses one. */
const OWN: readonly string[] = Object.keys(MOTION_STYLES);

/** A plugin's style for the mark, as the window's own are written. */
export function pluginTabStyle(motion: MotionDef): MotionStyle | undefined {
  if (!("highlight" in motion.spec)) return undefined;
  const highlight = motion.spec.highlight;
  const spring = (pair: readonly [number, number]): Spring => [pair[0], pair[1]];
  const slide =
    highlight.slide === "spring"
      ? ("spring" as const)
      : {
          duration: highlight.slide.duration,
          easing: easingOf(highlight.slide.easing, highlight.slide.duration).easing,
        };
  const enter = highlight.enter;
  return {
    shape: highlight.shape,
    lead: spring(highlight.lead),
    trail: spring(highlight.trail),
    slide,
    enter: {
      keyframes: enter.keyframes as Keyframe[],
      ...(enter.spring ? { spring: spring(enter.spring) } : {}),
      ...(enter.duration === undefined ? {} : { duration: enter.duration }),
    },
    renamed: highlight.renamed === null ? null : (highlight.renamed as Keyframe[]),
  };
}

/** The style the settings choose: one of the window's own by name, or a plugin's. */
export function tabStyle(
  setting: TabHighlight | Contributed,
  motions: readonly MotionDef[] | undefined,
): TabHighlight | MotionStyle {
  if (OWN.includes(setting)) return setting as TabHighlight;
  const found = motions?.find((motion) => motion.id === setting && motion.slot === "tab.highlight");
  return (found && pluginTabStyle(found)) ?? "glow";
}

/**
 * The window's own style a style looks like, for the style sheet: a plugin's is drawn
 * as the window's style of the same shape.
 */
export function tabStyleName(style: TabHighlight | MotionStyle): TabHighlight {
  if (typeof style === "string") return style;
  switch (style.shape) {
    case "glow":
      return "glow";
    case "underline":
      return "stretch";
    case "pill":
      return style.slide === "spring" ? "spring" : "glide";
  }
}
