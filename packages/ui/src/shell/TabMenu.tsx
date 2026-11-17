/**
 * The menu of one tab, opened with the other button on the tab or the menu key: its
 * name and colour, its group, where it is among the tabs, and closing it and the tabs
 * beside it.
 *
 * "Move into another tab" shows, in the menu's place, the tabs the tab's panes can be
 * moved into, with a row that goes back.
 */

import { useState } from "react";
import { useStore } from "zustand";

import {
  selectGroups,
  selectGroupTabs,
  selectTabOrder,
  selectTabPanes,
} from "../layout/selectors.ts";
import { TAB_COLORS, type TabId } from "../layout/types.ts";
import { AnchoredMenu } from "./AnchoredMenu.tsx";
import { useShell } from "./context.ts";
import { describeTabNow } from "./describing.ts";
import { menuHint } from "./hints.ts";
import { GroupGlyph, MenuGlyph } from "./icons.tsx";
import { Menu, type MenuItem } from "./Menu.tsx";
import { TAB_COLOR_NAMES, tabColorValue, UNNAMED_GROUP } from "./tab-looks.ts";
import type { Anchor } from "./tab-ui.ts";

/** How wide the menu is. */
const TAB_MENU_WIDTH = 290;

/** A dot of a colour, for a row that names a group or a tab. */
function Dot({ color }: { color: string | null }) {
  return (
    <span
      className="menu-dot"
      data-none={color === null || undefined}
      style={color === null ? undefined : { background: color }}
    />
  );
}

export function TabMenu({
  tab,
  anchor,
  keyboard,
}: {
  tab: TabId;
  anchor: Anchor;
  keyboard: boolean;
}) {
  const app = useShell();
  const [step, setStep] = useState<"options" | "move">("options");
  const view = useStore(app.config, (state) => state.view);
  // The menu says what is so as it opens, and again whenever the layout changes.
  const state = useStore(app.layout, (layout) => layout);
  const entry = state.tabs[tab];
  if (!entry) return null;

  const ui = app.tabUi.getState();
  const close = () => {
    ui.close();
    app.views.focus();
  };
  const run = (command: string, args: unknown = { tab }) => app.commands.run(command, args);
  const hint = (command: string) => menuHint(view, command);
  const order = selectTabOrder(state, entry.window);
  const at = order.indexOf(tab);
  const closable = (tabs: readonly TabId[]) => tabs.filter((other) => !state.tabs[other]?.locked);
  const leftOf = closable(order.slice(0, at));
  const rightOf = closable(order.slice(at + 1));
  const groups = selectGroups(state, entry.window);
  const own = groups.find((group) => group.id === entry.group);
  const others = groups.filter((group) => group.id !== entry.group);

  const options: MenuItem[] = [
    {
      key: "rename",
      label: "Rename tab…",
      icon: <MenuGlyph name="pencil" />,
      hint: hint("lume.tab.rename-start"),
      choose: () => ui.startRename(tab),
    },
    {
      key: "color",
      kind: "swatches",
      label: "Color",
      icon: <MenuGlyph name="palette" />,
      swatches: [
        {
          key: "none",
          label: "No color",
          color: null,
          checked: entry.color === null,
          choose: () => run("lume.tab.color", { tab, color: null }),
        },
        ...TAB_COLORS.map((color) => ({
          key: color,
          label: TAB_COLOR_NAMES[color],
          color: tabColorValue(color),
          checked: entry.color === color,
          choose: () => run("lume.tab.color", { tab, color }),
        })),
      ],
      choose: () => undefined,
    },
    {
      key: "new-group",
      label: own ? "Move to new group" : "Add tab to new group",
      icon: <MenuGlyph name="square-plus" />,
      separated: true,
      choose: () => {
        // The new group's editor opens once its label is on show.
        ui.awaitEditor(tab);
        run("lume.group.new");
      },
    },
  ];
  if (others.length > 0) {
    options.push({
      key: "to-group",
      kind: "heading",
      label: own ? "Move to group" : "Add to group",
      choose: () => undefined,
    });
    for (const group of others) {
      options.push({
        key: `group:${group.id}`,
        label: group.name ?? UNNAMED_GROUP,
        icon:
          group.icon === undefined ? (
            <Dot color={tabColorValue(group.color)} />
          ) : (
            <span className="menu-group-icon" style={{ color: tabColorValue(group.color) }}>
              <GroupGlyph icon={group.icon} size={11} />
            </span>
          ),
        hint: String(selectGroupTabs(state, group.id, entry.window).length),
        choose: () => run("lume.group.add", { tab, group: group.id }),
      });
    }
  }
  if (own) {
    const alone = selectGroupTabs(state, own.id, entry.window).length === 1;
    options.push({
      key: "remove-from-group",
      label: "Remove from group",
      icon: <MenuGlyph name="ungroup" />,
      choose: () => {
        // The last tab leaving takes the group with it: its label and line draw back first.
        const leave = () => run("lume.group.remove");
        const row = app.tabUi.getState().row;
        if (alone && row) void row.groupExit(own.id).then(leave);
        else leave();
      },
    });
  }
  options.push(
    {
      key: "new-tab",
      label: "New tab",
      icon: <MenuGlyph name="plus" />,
      hint: hint("lume.tab.new"),
      separated: true,
      choose: () => run("lume.tab.new", null),
    },
    {
      key: "move-left",
      label: "Move tab left",
      icon: <MenuGlyph name="chevron-left" />,
      disabled: at <= 0,
      hint: hint("lume.tab.move-left"),
      choose: () => run("lume.tab.move-left"),
    },
    {
      key: "move-right",
      label: "Move tab right",
      icon: <MenuGlyph name="chevron-right" />,
      disabled: at < 0 || at >= order.length - 1,
      hint: hint("lume.tab.move-right"),
      choose: () => run("lume.tab.move-right"),
    },
    {
      key: "move-into",
      label: "Move into another tab…",
      icon: <MenuGlyph name="columns-2" />,
      disabled: order.length < 2,
      hint: "›",
      stay: true,
      choose: () => setStep("move"),
    },
    {
      key: "lock",
      label: entry.locked ? "Unlock session" : "Lock session",
      icon: <MenuGlyph name="lock" />,
      separated: true,
      choose: () => run("lume.tab.lock"),
    },
    {
      key: "close-left",
      label: "Close tabs to the left",
      icon: <MenuGlyph name="chevron-first" />,
      disabled: leftOf.length === 0,
      ...(leftOf.length === 0 ? {} : { hint: String(leftOf.length) }),
      choose: () => run("lume.tab.close-left"),
    },
    {
      key: "close-right",
      label: "Close tabs to the right",
      icon: <MenuGlyph name="chevron-last" />,
      disabled: rightOf.length === 0,
      ...(rightOf.length === 0 ? {} : { hint: String(rightOf.length) }),
      choose: () => run("lume.tab.close-right"),
    },
    {
      key: "close",
      label: "Close tab",
      icon: <MenuGlyph name="x" />,
      disabled: entry.locked,
      hint: hint("lume.tab.close"),
      choose: () => run("lume.tab.close"),
    },
  );

  const title = describeTabNow(app, tab).label;
  const moving: MenuItem[] = [
    {
      key: "back",
      label: `Move ${title} into…`,
      icon: <MenuGlyph name="chevron-left" />,
      stay: true,
      choose: () => setStep("options"),
    },
    ...order
      .filter((other) => other !== tab)
      .map((other, index): MenuItem => {
        const target = state.tabs[other];
        const panes = selectTabPanes(state, other).length;
        return {
          key: `into:${other}`,
          label: describeTabNow(app, other).label,
          icon: <Dot color={target?.color ? tabColorValue(target.color) : null} />,
          ...(panes > 1 ? { hint: `${panes} panes` } : {}),
          separated: index === 0,
          choose: () => run("lume.tab.merge", { tab, into: other }),
        };
      }),
    {
      key: "explained",
      kind: "note",
      label:
        "The tab closes and opens as a split pane next to the panes already in the tab you pick.",
      choose: () => undefined,
    },
  ];

  return (
    <AnchoredMenu anchor={anchor} width={TAB_MENU_WIDTH} className="tab-menu">
      <Menu
        key={step}
        label={step === "move" ? "Move into another tab" : "Tab options"}
        items={step === "move" ? moving : options}
        close={close}
        // In the second step Escape goes back to the first.
        {...(step === "move" ? { escape: () => setStep("options") } : {})}
        keyboard={keyboard || step === "move"}
      />
    </AnchoredMenu>
  );
}
