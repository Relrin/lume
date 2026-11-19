/**
 * A menu or a small editor that opens from something in the window that is not a
 * control of its own, such as a tab: under it, or above it for something along the
 * bottom of the window, kept inside the window, growing from where it was opened.
 */

import type { CSSProperties, ReactNode } from "react";

import type { Anchor } from "./tab-ui.ts";

/** How far from the window's edges what opens is kept. */
const EDGE_PX = 8;

/** How far right of its anchor's start what opens starts. */
const NUDGE_PX = -6;

/**
 * The place what opens is put in, under its anchor or, with `above`, over it. `width`
 * is how wide it will be.
 */
export function AnchoredMenu({
  anchor,
  width,
  above = false,
  className,
  children,
}: {
  anchor: Anchor;
  width: number;
  above?: boolean;
  className?: string;
  children: ReactNode;
}) {
  const room = window.innerWidth;
  const left = Math.round(
    Math.max(EDGE_PX, Math.min(anchor.left + NUDGE_PX, room - width - EDGE_PX)),
  );
  // It grows from the point beside the anchor it was opened at.
  const origin = Math.max(12, Math.round(anchor.left - left + 16));
  const reach = above ? anchor.top : window.innerHeight - anchor.bottom;
  const style = {
    left: `${left}px`,
    top: `${Math.round(above ? anchor.top : anchor.bottom)}px`,
    "--anchored-origin": `${origin}px ${above ? "100%" : "0"}`,
    "--anchored-room": `${Math.max(120, Math.round(reach - 2 * EDGE_PX))}px`,
    "--anchored-width": `${width}px`,
  } as CSSProperties;
  return (
    <div
      className={className ? `anchored ${className}` : "anchored"}
      data-above={above ? true : undefined}
      style={style}
    >
      {children}
    </div>
  );
}
