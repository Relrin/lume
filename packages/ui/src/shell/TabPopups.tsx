/**
 * The menu of a tab, of a pane or of a terminal, or the editor of a group, whichever is open, wherever
 * the tabs are listed; and the editor of a group just made, once its label is on show.
 */

import { useEffect } from "react";
import { useStore } from "zustand";

import { selectGroupTabs } from "../layout/selectors.ts";
import { GROUP_EDITOR_DELAY_MS } from "../tuning.ts";
import { useShell } from "./context.ts";
import { GroupEditor } from "./GroupEditor.tsx";
import { anchorOf, labelId } from "./tab-ui.ts";
import { PaneMenu } from "./PaneMenu.tsx";
import { TabMenu } from "./TabMenu.tsx";
import { TerminalMenu } from "./TerminalMenu.tsx";

export function TabPopups() {
  const app = useShell();
  const popup = useStore(app.tabUi, (state) => state.popup);

  // A tab put in a new group: once the layout has the group, and its label has grown
  // in, the group's editor opens with its name to be typed.
  useEffect(() => {
    let timer: number | null = null;
    // The group the tab was in when it was asked for a new one, which is not the new one.
    let from: { tab: string; group: string | null } | null = null;
    const look = () => {
      const tab = app.tabUi.getState().editorFor;
      if (tab === null) from = null;
      if (tab === null || timer !== null) return;
      const state = app.layout.getState();
      const group = state.tabs[tab]?.group ?? null;
      if (from?.tab !== tab) from = { tab, group };
      if (group === null || group === from.group) return;
      if (selectGroupTabs(state, group).length !== 1) return;
      app.tabUi.getState().awaitEditor(null);
      timer = window.setTimeout(() => {
        timer = null;
        const label = document.querySelector(`[data-tab-id="${labelId(group)}"]`);
        if (!label) return;
        app.tabUi
          .getState()
          .open({ kind: "group", group, anchor: anchorOf(label), focusName: true });
      }, GROUP_EDITOR_DELAY_MS);
    };
    const stops = [app.layout.subscribe(look), app.tabUi.subscribe(look)];
    return () => {
      stops.forEach((stop) => stop());
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [app]);

  if (!popup) return null;
  if (popup.kind === "pane") {
    return (
      <PaneMenu
        key={popup.pane}
        pane={popup.pane}
        anchor={popup.anchor}
        keyboard={popup.keyboard}
      />
    );
  }
  if (popup.kind === "terminal") {
    return (
      <TerminalMenu
        key={`${popup.pane}:${popup.anchor.left}:${popup.anchor.top}`}
        pane={popup.pane}
        anchor={popup.anchor}
        selection={popup.selection}
      />
    );
  }
  return popup.kind === "tab" ? (
    <TabMenu key={popup.tab} tab={popup.tab} anchor={popup.anchor} keyboard={popup.keyboard} />
  ) : (
    <GroupEditor
      key={popup.group}
      group={popup.group}
      anchor={popup.anchor}
      focusName={popup.focusName}
    />
  );
}
