/**
 * The menu of one pane's terminal, opened with the other button in it: what is
 * selected copied, the clipboard pasted, and everything selected.
 *
 * A program that follows the mouse gets the other button itself; there the menu opens
 * with Shift held.
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
const TERMINAL_MENU_WIDTH = 220;

export function TerminalMenu({
  pane,
  anchor,
  selection,
}: {
  pane: PaneId;
  anchor: Anchor;
  /** Whether text was selected when the menu was asked for. */
  selection: boolean;
}) {
  const app = useShell();
  const view = useStore(app.config, (state) => state.view);
  const close = () => {
    app.tabUi.getState().close();
    app.views.focus();
  };
  const run = (command: string) => app.commands.run(command, { pane });
  const items: MenuItem[] = [
    {
      key: "copy",
      label: "Copy",
      icon: <MenuGlyph name="copy" />,
      hint: menuHint(view, "lume.terminal.copy"),
      disabled: !selection,
      choose: () => run("lume.terminal.copy"),
    },
    {
      key: "paste",
      label: "Paste",
      icon: <MenuGlyph name="clipboard" />,
      hint: menuHint(view, "lume.terminal.paste"),
      choose: () => run("lume.terminal.paste"),
    },
    {
      key: "select-all",
      label: "Select all",
      icon: <MenuGlyph name="square-dashed-text" />,
      hint: menuHint(view, "lume.terminal.select-all"),
      separated: true,
      choose: () => run("lume.terminal.select-all"),
    },
  ];
  return (
    <AnchoredMenu anchor={anchor} width={TERMINAL_MENU_WIDTH} className="terminal-menu">
      <Menu label="Terminal" items={items} close={close} keyboard={false} />
    </AnchoredMenu>
  );
}
