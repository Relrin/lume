/**
 * The window's tabs as a list down the side of the window, for a window that has
 * them there in place of the row along the top.
 *
 * Each row is a tab as the top bar shows one, laid out for a column: pressed to show
 * the tab, dragged up or down to another place, renamed in place, closed by its
 * button or the middle button. With the keyboard on a tab, the up and down arrows
 * move it to the tab's neighbours.
 */

import { useStore } from "zustand";

import { NEW_TAB_DROP } from "../drag/controller.ts";
import { selectActiveTab, selectTabOrder } from "../layout/selectors.ts";
import type { TabId } from "../layout/types.ts";
import { useShell } from "./context.ts";
import { NewTabButton } from "./NewTabButton.tsx";
import { TabItem } from "./TabStrip.tsx";

const NO_TABS: readonly TabId[] = [];

/** The tabs, one under another, and a row that opens another. */
export function TabListPanel() {
  const app = useShell();
  const tabs = useStore(app.layout, (state) => (state.ready ? selectTabOrder(state) : NO_TABS));
  const active = useStore(app.layout, (state) => (state.ready ? selectActiveTab(state) : null));
  return (
    <div className="tab-column" data-drop={NEW_TAB_DROP}>
      <div className="tab-list" role="tablist" aria-label="Tabs" aria-orientation="vertical">
        {tabs.map((tab, index) => (
          <TabItem key={tab} tab={tab} active={tab === active} index={index} vertical />
        ))}
      </div>
      <NewTabButton column />
    </div>
  );
}
