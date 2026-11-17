/** The Connect palette's picture of each kind of profile, drawn with the shell's icons. */

import { GlobeIcon, LayersIcon, TerminalIcon } from "../shell/icons.tsx";
import type { ConnectKind } from "./model.ts";

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
