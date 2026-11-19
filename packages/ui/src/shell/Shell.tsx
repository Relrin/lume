/**
 * The window: a bar along the top, the panes of the tab on show in the middle, and
 * whatever panels are docked at the left, the right and the bottom around them, all
 * of it over the picture the configuration puts behind the panes, if it names one.
 */

import { useStore } from "zustand";

import { ConnectPalette } from "../connect/ConnectPalette.tsx";
import { Palette } from "../palette/Palette.tsx";
import { Prompts } from "../Prompts.tsx";
import { tabStyle, tabStyleName } from "../motion/tab.ts";
import { Background } from "./Background.tsx";
import { CursorFxLayer } from "./CursorFxLayer.tsx";
import type { ShellApp } from "./boot.ts";
import { ShellContext } from "./context.ts";
import { DockArea, DockDropPreview } from "./DockArea.tsx";
import { Notices } from "./Notices.tsx";
import { TabArea } from "./TabArea.tsx";
import { TabPopups } from "./TabPopups.tsx";
import { TopBar } from "./TopBar.tsx";

/** The whole of a terminal window. `app` is what was started for the page. */
export function Shell({ app }: { app: ShellApp }) {
  // Which program tiles keep their colours: the style sheet greys the others.
  const tileColor = useStore(
    app.config,
    (state) => state.view?.config.appearance["tile-color"] ?? "always",
  );
  // How the mark of the tab on show moves: the style sheet and the row of tabs read it.
  const setting = useStore(
    app.config,
    (state) => state.view?.config.appearance["tab-highlight"] ?? "glow",
  );
  const motions = useStore(app.plugins, (state) => state.shown?.motions);
  const tabHighlight = tabStyleName(tabStyle(setting, motions));
  return (
    <ShellContext value={app}>
      <div className="shell" data-tile-color={tileColor} data-tab-highlight={tabHighlight}>
        <Background />
        <TopBar />
        <div className="shell-body">
          <DockArea area="left" />
          <div className="shell-centre">
            <TabArea />
            <DockArea area="bottom" />
          </div>
          <DockArea area="right" />
          <DockDropPreview />
        </div>
        <CursorFxLayer />
        <TabPopups />
        <Palette />
        <ConnectPalette />
        <Notices store={app.notices} run={(command, args) => app.commands.run(command, args)} />
        <Prompts store={app.prompts} />
      </div>
    </ShellContext>
  );
}
