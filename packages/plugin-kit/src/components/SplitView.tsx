/**
 * Two parts of a view side by side, or one above the other, with a line between them
 * that is dragged to give one more room.
 */

import { type KeyboardEvent, type PointerEvent, type ReactNode, useRef, useState } from "react";

import { cx } from "./icons";

/** Holds a share between its least and its most. */
export function clampShare(share: number, min: number, max: number): number {
  if (Number.isNaN(share)) return min;
  return Math.min(max, Math.max(min, share));
}

export interface SplitViewProps {
  /** `row` puts the parts side by side, `column` one above the other. */
  readonly direction?: "row" | "column" | undefined;
  /** The first part's share of the room, from 0 to 1. */
  readonly share: number;
  /** Called while the line is dragged, with the first part's new share. */
  readonly onShareChange: (share: number) => void;
  /** The least and the most the first part may have. */
  readonly min?: number | undefined;
  readonly max?: number | undefined;
  readonly first: ReactNode;
  readonly second: ReactNode;
  /** What the line divides, for assistive technology. */
  readonly label?: string | undefined;
  readonly className?: string | undefined;
}

/** Two parts with a line between them that can be dragged, or moved with the arrow keys. */
export function SplitView({
  direction = "row",
  share,
  onShareChange,
  min = 0.15,
  max = 0.85,
  first,
  second,
  label = "Resize",
  className,
}: SplitViewProps) {
  const box = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState(false);
  const held = clampShare(share, min, max);
  const row = direction === "row";

  const moved = (event: PointerEvent) => {
    const bounds = box.current?.getBoundingClientRect();
    if (!dragging || !bounds) return;
    const next = row
      ? (event.clientX - bounds.left) / bounds.width
      : (event.clientY - bounds.top) / bounds.height;
    onShareChange(clampShare(next, min, max));
  };

  const keyed = (event: KeyboardEvent) => {
    const less = row ? "ArrowLeft" : "ArrowUp";
    const more = row ? "ArrowRight" : "ArrowDown";
    if (event.key !== less && event.key !== more) return;
    event.preventDefault();
    onShareChange(clampShare(held + (event.key === more ? 0.02 : -0.02), min, max));
  };

  return (
    <div
      ref={box}
      className={cx("lk-split", row ? "lk-split-row" : "lk-split-column", className)}
      data-dragging={dragging || undefined}
    >
      <div className="lk-split-part" style={{ flex: `${held} 1 0` }}>
        {first}
      </div>
      <div
        role="separator"
        aria-label={label}
        aria-orientation={row ? "vertical" : "horizontal"}
        aria-valuemin={Math.round(min * 100)}
        aria-valuemax={Math.round(max * 100)}
        aria-valuenow={Math.round(held * 100)}
        tabIndex={0}
        className="lk-split-line"
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture(event.pointerId);
          setDragging(true);
          event.preventDefault();
        }}
        onPointerMove={moved}
        onPointerUp={() => setDragging(false)}
        onPointerCancel={() => setDragging(false)}
        onKeyDown={keyed}
      />
      <div className="lk-split-part" style={{ flex: `${1 - held} 1 0` }}>
        {second}
      </div>
    </div>
  );
}
