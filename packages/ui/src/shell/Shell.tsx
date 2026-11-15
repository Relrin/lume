/**
 * The window: a bar with the tabs, and under it the panes of the tab on show.
 */

import { Prompts } from "../Prompts.tsx";
import type { ShellApp } from "./boot.ts";
import { ShellContext } from "./context.ts";
import { LumeMark } from "./icons.tsx";
import { TabArea } from "./TabArea.tsx";
import { TabStrip } from "./TabStrip.tsx";

/** The whole of a terminal window. `app` is what was started for the page. */
export function Shell({ app }: { app: ShellApp }) {
  return (
    <ShellContext value={app}>
      <div className="shell">
        <header className="top-bar">
          <span className="top-mark">
            <LumeMark />
          </span>
          <TabStrip />
        </header>
        <TabArea />
        <Prompts store={app.prompts} />
      </div>
    </ShellContext>
  );
}
