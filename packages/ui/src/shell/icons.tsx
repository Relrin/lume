/**
 * The shell's icons: small line drawings in the colour of the text around them.
 *
 * Each is decoration beside a label or inside a button that names itself, so none is
 * announced on its own.
 */

import type { ReactNode } from "react";

function Icon({ size, weight, children }: { size: number; weight: number; children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={weight}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

/** A shell's prompt: what a terminal is. */
export function TerminalIcon({ size = 13 }: { size?: number }) {
  return (
    <Icon size={size} weight={2}>
      <polyline points="4 17 10 11 4 5" />
      <line x1="12" y1="19" x2="20" y2="19" />
    </Icon>
  );
}

export function PlusIcon({ size = 14 }: { size?: number }) {
  return (
    <Icon size={size} weight={2}>
      <line x1="12" y1="5" x2="12" y2="19" />
      <line x1="5" y1="12" x2="19" y2="12" />
    </Icon>
  );
}

export function CloseIcon({ size = 13 }: { size?: number }) {
  return (
    <Icon size={size} weight={2}>
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </Icon>
  );
}

/** A pane divided into a left and a right half. */
export function SplitRightIcon({ size = 14 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.8}>
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <line x1="12" y1="3" x2="12" y2="21" />
    </Icon>
  );
}

/** A pane divided into an upper and a lower half. */
export function SplitDownIcon({ size = 14 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.8}>
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <line x1="3" y1="12" x2="21" y2="12" />
    </Icon>
  );
}

/** Two arrows pointing apart: a pane grown to fill its tab. */
export function ZoomIcon({ size = 13 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.8}>
      <polyline points="15 3 21 3 21 9" />
      <polyline points="9 21 3 21 3 15" />
      <line x1="21" y1="3" x2="14" y2="10" />
      <line x1="3" y1="21" x2="10" y2="14" />
    </Icon>
  );
}

/** A ring with a dot: the application's mark. */
export function LumeMark({ size = 18 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="2" />
      <circle cx="12" cy="12" r="3.5" fill="currentColor" />
    </svg>
  );
}
