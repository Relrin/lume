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

/** Two overlapping squares: a pane lifted off the others. */
export function FloatIcon({ size = 14 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.8}>
      <rect x="8" y="8" width="13" height="13" rx="2" />
      <path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3" />
    </Icon>
  );
}

/** A line along the bottom: a pane put away at the corner. */
export function PutAwayIcon({ size = 13 }: { size?: number }) {
  return (
    <Icon size={size} weight={2}>
      <line x1="5" y1="18" x2="19" y2="18" />
    </Icon>
  );
}

/** Three lines: the application's menu. */
export function MenuIcon({ size = 16 }: { size?: number }) {
  return (
    <Icon size={size} weight={2}>
      <line x1="4" y1="7" x2="20" y2="7" />
      <line x1="4" y1="12" x2="20" y2="12" />
      <line x1="4" y1="17" x2="20" y2="17" />
    </Icon>
  );
}

export function ChevronDownIcon({ size = 12 }: { size?: number }) {
  return (
    <Icon size={size} weight={2.2}>
      <polyline points="6 9 12 15 18 9" />
    </Icon>
  );
}

export function CheckIcon({ size = 13 }: { size?: number }) {
  return (
    <Icon size={size} weight={2.2}>
      <polyline points="5 12 10 17 19 7" />
    </Icon>
  );
}

/** A line near the bottom: a window put away. */
export function MinimiseIcon({ size = 12 }: { size?: number }) {
  return (
    <Icon size={size} weight={2}>
      <line x1="5" y1="12" x2="19" y2="12" />
    </Icon>
  );
}

/** A square: a window grown to fill the screen. */
export function MaximiseIcon({ size = 11 }: { size?: number }) {
  return (
    <Icon size={size} weight={2}>
      <rect x="4" y="4" width="16" height="16" rx="2" />
    </Icon>
  );
}

/** A square in front of another: a window put back to the size it had. */
export function RestoreIcon({ size = 12 }: { size?: number }) {
  return (
    <Icon size={size} weight={2}>
      <rect x="4" y="8" width="12" height="12" rx="2" />
      <path d="M8 8V6a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-2" />
    </Icon>
  );
}

/** A bar folded against an edge: a panel put away. */
export function CollapseIcon({ size = 13 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.9}>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <line x1="9" y1="4" x2="9" y2="20" />
    </Icon>
  );
}

export function SearchIcon({ size = 15 }: { size?: number }) {
  return (
    <Icon size={size} weight={2}>
      <circle cx="11" cy="11" r="6.5" />
      <line x1="20" y1="20" x2="16" y2="16" />
    </Icon>
  );
}

/** A dot with rays about it: how the application as a whole behaves. */
export function GeneralIcon({ size = 15 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.8}>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 2v3M12 19v3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M2 12h3M19 12h3M4.9 19.1 7 17M17 7l2.1-2.1" />
    </Icon>
  );
}

/** A disc, one half of it filled: how things look. */
export function AppearanceIcon({ size = 15 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.8}>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 3a9 9 0 0 0 0 18z" fill="currentColor" />
    </Icon>
  );
}

/** A line that forks: a branch of a repository. */
export function BranchIcon({ size = 13 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.9}>
      <line x1="6" y1="4" x2="6" y2="15" />
      <circle cx="6" cy="18" r="3" />
      <circle cx="18" cy="7" r="3" />
      <path d="M18 10a8 8 0 0 1-8 8" />
    </Icon>
  );
}

/** A heartbeat: something running. */
export function PulseIcon({ size = 13 }: { size?: number }) {
  return (
    <Icon size={size} weight={2}>
      <polyline points="3 12 7 12 10 4 14 20 17 12 21 12" />
    </Icon>
  );
}

export function FolderIcon({ size = 13 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.9}>
      <path d="M3 7a2 2 0 0 1 2-2h4l2 2.5h8a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    </Icon>
  );
}

/** A triangle with a mark in it: something that wants attention. */
export function AlertIcon({ size = 13 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.9}>
      <path d="M12 4 3 19h18z" />
      <line x1="12" y1="10" x2="12" y2="14" />
      <line x1="12" y1="16.5" x2="12" y2="16.6" />
    </Icon>
  );
}

/** A filled dot. */
export function DotIcon({ size = 9 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="9" fill="currentColor" />
    </svg>
  );
}
