/**
 * The window: a bar along the top, the panes of the tab on show in the middle, and
 * whatever panels are docked at the left, the right and the bottom around them, all
 * of it over the picture the configuration puts behind the panes, if it names one.
 */

import { Palette } from "../palette/Palette.tsx";
import { Prompts } from "../Prompts.tsx";
import { Background } from "./Background.tsx";
import type { ShellApp } from "./boot.ts";
import { ShellContext } from "./context.ts";
import { DockArea, DockDropPreview } from "./DockArea.tsx";
import { Notices } from "./Notices.tsx";
import { TabArea } from "./TabArea.tsx";
import { TopBar } from "./TopBar.tsx";

/** The whole of a terminal window. `app` is what was started for the page. */
export function Shell({ app }: { app: ShellApp }) {
  return (
    <ShellContext value={app}>
      <div className="shell">
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
        <Palette />
        <Notices store={app.notices} />
        <Prompts store={app.prompts} />
      </div>
    </ShellContext>
  );
}
