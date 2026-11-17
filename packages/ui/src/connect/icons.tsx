/**
 * The Connect palette's own icons: small line drawings in the colour of the text
 * around them. Each is decoration beside a label, so none is announced on its own.
 */

import type { ReactNode } from "react";

import { TerminalIcon } from "../shell/icons.tsx";
import type { ConnectKind } from "./model.ts";

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

/** A globe: another machine. */
export function GlobeIcon({ size = 15 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.8}>
      <circle cx="12" cy="12" r="9" />
      <line x1="3" y1="12" x2="21" y2="12" />
      <path d="M12 3a14 14 0 0 1 0 18a14 14 0 0 1 0-18" />
    </Icon>
  );
}

/** Stacked layers: a system inside this one. */
export function LayersIcon({ size = 15 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.8}>
      <polygon points="12 3 21 8 12 13 3 8 12 3" />
      <polyline points="3 13 12 18 21 13" />
    </Icon>
  );
}

/** A page with a folded corner: a file to edit. */
export function FileIcon({ size = 15 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.8}>
      <path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
      <polyline points="14 3 14 9 20 9" />
    </Icon>
  );
}

/** Two arrows chasing each other: look again. */
export function RefreshIcon({ size = 15 }: { size?: number }) {
  return (
    <Icon size={size} weight={1.8}>
      <polyline points="21 4 21 10 15 10" />
      <polyline points="3 20 3 14 9 14" />
      <path d="M5.6 9a7 7 0 0 1 12.2-2.4L21 10M3 14l3.2 3.4A7 7 0 0 0 18.4 15" />
    </Icon>
  );
}

/** The icon of a kind of profile. */
export function KindIcon({ kind, size = 15 }: { kind: ConnectKind; size?: number }) {
  switch (kind) {
    case "ssh":
      return <GlobeIcon size={size} />;
    case "wsl":
      return <LayersIcon size={size} />;
    case "local":
      return <TerminalIcon size={size} />;
  }
}
