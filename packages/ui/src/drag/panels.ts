/**
 * Dragging a panel from one side of the window to another.
 *
 * Which panel is in hand and which side it is held over are kept here, for the one
 * element that shows where the panel would be docked. The side is worked out from
 * where the pointer is in the window, not from what lies under it: a side that has no
 * panel yet has no element to hold anything over.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { Rect } from "../layout/geometry.ts";
import type { SIDE_AREAS } from "../layout/selectors.ts";
import type { PanelId } from "../layout/types.ts";
import { PANEL_DROP_FRACTION } from "../tuning.ts";

/** A side of the window a panel can be docked at. */
export type SideArea = (typeof SIDE_AREAS)[number];

/** The panel that is being dragged, and the side it would be docked at. */
export interface PanelDragState {
  readonly panel: PanelId | null;
  readonly target: SideArea | null;
  /** Records the panel in hand and the side it is held over. */
  hold(panel: PanelId, target: SideArea | null): void;
  /** The panel was let go. */
  drop(): void;
}

/** The store a window keeps its panel drag in. */
export type PanelDragStore = StoreApi<PanelDragState>;

/** Makes a window's panel drag store. Nothing is in hand to begin with. */
export function createPanelDragStore(): PanelDragStore {
  return createStore<PanelDragState>()((set, get) => ({
    panel: null,
    target: null,
    hold: (panel, target) => {
      const held = get();
      if (held.panel !== panel || held.target !== target) set({ panel, target });
    },
    drop: () => {
      if (get().panel !== null) set({ panel: null, target: null });
    },
  }));
}

/**
 * The side of `body` a panel held at a point would be docked at, or nothing when the
 * point is near none of the sides it may go to. The left and the right come before the
 * bottom, so the lower corners belong to the columns.
 */
export function panelDropArea(
  point: { readonly x: number; readonly y: number },
  body: Rect,
  allowed: readonly SideArea[],
  fraction: number = PANEL_DROP_FRACTION,
): SideArea | null {
  if (body.width <= 0 || body.height <= 0) return null;
  const across = (point.x - body.x) / body.width;
  const down = (point.y - body.y) / body.height;
  if (across < 0 || across > 1 || down < 0 || down > 1) return null;
  const may = (area: SideArea) => allowed.includes(area);
  if (across < fraction && may("left")) return "left";
  if (across > 1 - fraction && may("right")) return "right";
  if (down > 1 - fraction && may("bottom")) return "bottom";
  return null;
}
