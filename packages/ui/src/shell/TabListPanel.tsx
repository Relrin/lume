/**
 * The window's tabs as a list down the side of the window, for a window that has
 * them there in place of the row along the top.
 *
 * Each row is a tab as the top bar shows one, laid out for a column: pressed to show
 * the tab, dragged up or down to another place, renamed in place, closed by its
 * button or the middle button. A group's label heads its tabs, and folding the group
 * hides them. With the keyboard on a tab, the up and down arrows move it to the tab's
 * neighbours.
 */

import { NEW_TAB_DROP } from "../drag/controller.ts";
import { NewTabButton } from "./NewTabButton.tsx";
import { TabColumn } from "./TabStrip.tsx";

/** The tabs, one under another, and a row that opens another. */
export function TabListPanel() {
  return (
    <div className="tab-column" data-drop={NEW_TAB_DROP}>
      <TabColumn />
      <NewTabButton column />
    </div>
  );
}
