/**
 * The panes that float over the window's tabs.
 *
 * Each is a card like any other, with a header whatever the settings say and edges to
 * resize it by. They are rendered inside the element the tab's own cards are laid out
 * in, so that the two kinds share one space and one way of being placed.
 *
 * Which card is in front is a matter of how each is drawn, not of where it stands
 * among the others in the page: the cards are kept in one fixed order here, so that
 * bringing a card to the front never moves its element, and a pointer that has hold
 * of the card keeps hold of it.
 */

import { useMemo } from "react";
import { useStore } from "zustand";

import { selectFloating, selectMinimised } from "../layout/selectors.ts";
import type { PaneId } from "../layout/types.ts";
import { useShell } from "./context.ts";
import { PaneCard } from "./PaneCard.tsx";

/** No panes: one list for every window that has none floating. */
const NONE: readonly PaneId[] = [];

/** The cards of the panes floating over this window. */
export function FloatingLayer() {
  const app = useShell();
  const floating = useStore(app.layout, (state) => (state.ready ? selectFloating(state) : NONE));
  // A card belongs to one layout: see the tab area, which keys its cards the same way.
  const epoch = useStore(app.layout, (state) => state.epoch);
  // A pane that is put away has no card: it is a marker at the window's corner.
  const minimised = useStore(app.layout, (state) => (state.ready ? selectMinimised(state) : NONE));
  const fixed = useMemo(
    () => floating.filter((pane) => !minimised.includes(pane)).sort(),
    [floating, minimised],
  );
  return (
    <>
      {fixed.map((pane) => (
        <PaneCard key={`${epoch}:${pane}`} pane={pane} header floating />
      ))}
    </>
  );
}
