/**
 * The menu of one pane, opened with the other button on its header: the pane's session
 * shown in another place too, in a new tab or a new window.
 */

import { useStore } from "zustand";

import type { PaneId } from "../layout/types.ts";
import { AnchoredMenu } from "./AnchoredMenu.tsx";
import { useShell } from "./context.ts";
import { menuHint } from "./hints.ts";
import { MenuGlyph } from "./icons.tsx";
import { Menu, type MenuItem } from "./Menu.tsx";
import type { Anchor } from "./tab-ui.ts";

/** How wide the menu is. */
const PANE_MENU_WIDTH = 250;

export function PaneMenu({
  pane,
  anchor,
  keyboard,
}: {
  pane: PaneId;
  anchor: Anchor;
  keyboard: boolean;
}) {
  const app = useShell();
  const view = useStore(app.config, (state) => state.view);
  const content = useStore(app.layout, (state) => state.panes[pane]?.content);
  if (!content) return null;

  const close = () => {
    app.tabUi.getState().close();
    app.views.focus();
  };
  const run = (command: string) => app.commands.run(command, { pane });
  // Only a running session can be shown in a second place.
  const shown = content.type === "terminal" && content.session !== null;
  const items: MenuItem[] = [
    {
      key: "show-in-new-tab",
      label: "Show in new tab",
      icon: <MenuGlyph name="square-plus" />,
      hint: menuHint(view, "lume.pane.show-in-new-tab"),
      disabled: !shown,
      choose: () => run("lume.pane.show-in-new-tab"),
    },
    {
      key: "show-in-new-window",
      label: "Show in new window",
      icon: <MenuGlyph name="copy" />,
      hint: menuHint(view, "lume.pane.show-in-new-window"),
      disabled: !shown,
      choose: () => run("lume.pane.show-in-new-window"),
    },
  ];

  return (
    <AnchoredMenu anchor={anchor} width={PANE_MENU_WIDTH} className="pane-menu">
      <Menu label="Pane options" items={items} close={close} keyboard={keyboard} />
    </AnchoredMenu>
  );
}
