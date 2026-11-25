/**
 * The window: a bar along the top, the panes of the tab on show in the middle,
 * whatever panels are docked at the left, the right and the bottom around them, and
 * the status bar along the bottom while it has something to show, all of it over the
 * picture the configuration puts behind the panes, if it names one.
 *
 * Each of these is a part of its own: one that cannot be drawn says so in its place, and
 * what opens over the window and cannot be drawn is put away. The terminals and their
 * sessions are kept outside all of it, and carry on whatever cannot be drawn.
 */

import { useStore } from "zustand";

import { ConnectPalette } from "../connect/ConnectPalette.tsx";
import { connectionDialog } from "../connections/store.ts";
import { ErrorBoundary } from "../ErrorBoundary.tsx";
import { Palette } from "../palette/Palette.tsx";
import { Prompts } from "../Prompts.tsx";
import { tabStyle, tabStyleName } from "../motion/tab.ts";
import { Background } from "./Background.tsx";
import { ColorPickerHost } from "./ColorPicker.tsx";
import { Connection } from "./Connection.tsx";
import { CursorFxLayer } from "./CursorFxLayer.tsx";
import type { ShellApp } from "./boot.ts";
import { colorPicker } from "./color-picker.ts";
import { ShellContext } from "./context.ts";
import { DockArea, DockDropPreview } from "./DockArea.tsx";
import { Notices } from "./Notices.tsx";
import { folderDialog } from "./open-folder.ts";
import { OpenFolder } from "./OpenFolder.tsx";
import { FailedPart, Overlay, Part, Questions } from "./Part.tsx";
import { StatusBar } from "./StatusBar.tsx";
import { TabArea } from "./TabArea.tsx";
import { TabPopups } from "./TabPopups.tsx";
import { TopBar } from "./TopBar.tsx";
import { BareBar, WindowFailed } from "./WindowFailed.tsx";

/** The whole of a terminal window. `app` is what was started for the page. */
export function Shell({ app }: { app: ShellApp }) {
  return (
    <ShellContext value={app}>
      <ErrorBoundary
        fallback={(failure) => (
          <WindowFailed title="The window could not be drawn" failure={failure} bar={<BareBar />} />
        )}
      >
        <Parts app={app} />
      </ErrorBoundary>
    </ShellContext>
  );
}

/** The window as it is made of its parts. */
function Parts({ app }: { app: ShellApp }) {
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
  /** Puts away something that opens over the window, and gives the keyboard back. */
  const putAway = (store: { getState(): { close(): void } }) => () => {
    store.getState().close();
    app.views.focus();
  };
  return (
    <div className="shell" data-tile-color={tileColor} data-tab-highlight={tabHighlight}>
      <Part what="The background" quiet>
        <Background />
      </Part>
      <TopBar />
      <div className="shell-body">
        <Part what="The left panels">
          <DockArea area="left" />
        </Part>
        <div className="shell-centre">
          <ErrorBoundary
            fallback={(failure) => (
              <main className="tab-area">
                <FailedPart what="The panes" failure={failure} />
              </main>
            )}
          >
            <TabArea />
          </ErrorBoundary>
          <Part what="The bottom panels">
            <DockArea area="bottom" />
          </Part>
        </div>
        <Part what="The right panels">
          <DockArea area="right" />
        </Part>
        <Part what="Where a panel would be docked" quiet>
          <DockDropPreview />
        </Part>
      </div>
      <Part what="The status bar">
        <StatusBar />
      </Part>
      <Part what="The cursor's effects" quiet>
        <CursorFxLayer />
      </Part>
      <Overlay what="The menu" close={putAway(app.tabUi)} opens={app.tabUi.subscribe}>
        <TabPopups />
      </Overlay>
      <Overlay
        what="The command palette"
        close={putAway(app.palette)}
        opens={app.palette.subscribe}
      >
        <Palette />
      </Overlay>
      <Overlay
        what="The Connect palette"
        close={putAway(app.connect)}
        opens={app.connect.subscribe}
      >
        <ConnectPalette />
      </Overlay>
      <Overlay what="The colour picker" close={putAway(colorPicker)} opens={colorPicker.subscribe}>
        <ColorPickerHost />
      </Overlay>
      <Overlay
        what="The dialog that opens a folder"
        close={putAway(folderDialog)}
        opens={folderDialog.subscribe}
      >
        <OpenFolder />
      </Overlay>
      <Overlay
        what="The dialog that adds a connection"
        close={putAway(connectionDialog)}
        opens={connectionDialog.subscribe}
      >
        <Connection />
      </Overlay>
      <Part what="The notices" quiet>
        <Notices store={app.notices} run={(command, args) => app.commands.run(command, args)} />
      </Part>
      <Questions>
        <Prompts store={app.prompts} />
      </Questions>
    </div>
  );
}
