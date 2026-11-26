/**
 * The page's stores: what it knows and what the user is doing, before anything else is
 * made. None of them needs anything but the others.
 */

import { createCommandStore, type CommandStore } from "../../commands/store.ts";
import { createConfigStore, type ConfigStore } from "../../config/store.ts";
import { createConnectStore, type ConnectStore } from "../../connect/store.ts";
import { createPanelDragStore, type PanelDragStore } from "../../drag/panels.ts";
import { PaneReports } from "../../events/store.ts";
import { selectFocus } from "../../layout/selectors.ts";
import { createLayoutStore, type LayoutStore } from "../../layout/store.ts";
import type { PaneId } from "../../layout/types.ts";
import { createPaletteStore, type PaletteStore } from "../../palette/store.ts";
import { PaneLooks } from "../../panes/looks.ts";
import { PaneTitles } from "../../panes/titles.ts";
import { createPluginsStore, type PluginsStore } from "../../plugins/store.ts";
import { createProfilesStore, findProfile, type ProfilesStore } from "../../profiles/store.ts";
import { createRexStore, type RexStore } from "../../rex/store.ts";
import { createStatusStore, type StatusStore } from "../../status/registry.ts";
import type { Appearance } from "../../theme.ts";
import { createWindowStateStore, type WindowStateStore } from "../../window/state.ts";
import { paneFacts } from "../describing.ts";
import { createHostLinkStore, type HostLinkStore } from "../host-link.ts";
import { createTabUiStore, type TabUiStore } from "../tab-ui.ts";

export interface Stores {
  readonly layout: LayoutStore;
  readonly config: ConfigStore;
  readonly commandStore: CommandStore;
  readonly titles: PaneTitles;
  readonly palette: PaletteStore;
  readonly connect: ConnectStore;
  readonly profiles: ProfilesStore;
  readonly rex: RexStore;
  readonly status: StatusStore;
  readonly plugins: PluginsStore;
  readonly windowState: WindowStateStore;
  readonly hostLink: HostLinkStore;
  readonly reports: PaneReports;
  readonly looks: PaneLooks;
  readonly panelDrag: PanelDragStore;
  /** What the user is doing with the tabs and the panes' menus. The terminals open one too. */
  readonly tabUi: TabUiStore;
}

export function createStores(): Stores {
  const layout = createLayoutStore();
  const windowState = createWindowStateStore();
  // The user looks at the pane that has the keyboard, while the window is the one in use.
  const attended = (pane: PaneId): boolean => {
    const state = layout.getState();
    return windowState.getState().focused && state.ready && selectFocus(state) === pane;
  };
  return {
    layout,
    config: createConfigStore(),
    commandStore: createCommandStore(),
    titles: new PaneTitles(),
    palette: createPaletteStore(),
    connect: createConnectStore(),
    profiles: createProfilesStore(),
    rex: createRexStore(),
    status: createStatusStore(),
    plugins: createPluginsStore(),
    windowState,
    hostLink: createHostLinkStore(),
    reports: new PaneReports(attended),
    looks: new PaneLooks(),
    panelDrag: createPanelDragStore(),
    tabUi: createTabUiStore(),
  };
}

/**
 * The look of a pane's terminal: the window's, which `look` gives, or the one its
 * profile asks for.
 */
export function appearanceFor(
  { reports, profiles, layout, rex, looks }: Stores,
  look: () => Appearance,
): (pane: PaneId) => Appearance {
  return (pane) => {
    const report = reports.get(pane);
    const list = profiles.getState().list;
    // A session says what it was started from; a pane yet to have one runs what it
    // names, or the default.
    const id =
      report.session !== null
        ? report.profile
        : (paneFacts(layout.getState(), pane, rex.getState().list).profile ??
          list?.default ??
          undefined);
    return looks.of(look(), findProfile(list, id), list?.looks);
  };
}
