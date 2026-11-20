/**
 * The window's tabs, in a row along the top bar, and one tab as it is shown wherever
 * the tabs are listed.
 *
 * Which tabs there are, which is on show and how they are grouped come from the layout
 * and render here. What a tab is called does not: a tab without a name of its own is
 * called after the program in it, which may rename itself many times a second, so the
 * label is written straight to its element whenever it changes.
 *
 * A group's label stands in front of its tabs, and a line in the group's colour runs
 * under the label and the tabs. Pressing the label folds the group away, or opens it.
 * Tabs and whole groups are dragged to other places; a menu opens on a tab and an
 * editor on a label with the other button.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";
import { useStore } from "zustand";

import { NEW_TAB_DROP } from "../drag/controller.ts";
import { greatest, type Activity } from "../events/activity.ts";
import type { PaneIcon } from "../events/programs.ts";
import {
  selectActiveTab,
  selectStripKey,
  selectTabOrder,
  stripItems,
  type StripItem,
} from "../layout/selectors.ts";
import type { Group, GroupId, TabId } from "../layout/types.ts";
import { DRAG_SETTLE_TIMEOUT_MS, DRAG_START_PX } from "../tuning.ts";
import { tabStyle } from "../motion/tab.ts";
import type { ShellApp } from "./boot.ts";
import { useShell } from "./context.ts";
import { describeTabNow, followTab, mark } from "./describing.ts";
import { useKeyHint } from "./hints.ts";
import { BellOffIcon, CloseIcon, GroupGlyph, LockIcon, ProgramTile } from "./icons.tsx";
import { roveWithin } from "./roving.ts";
import {
  dropPlace,
  groupDropIntent,
  joinedGroup,
  tabDropIntent,
  type DropBox,
  type RowTab,
} from "./tab-drop.ts";
import { tabCount, UNNAMED_GROUP } from "./tab-looks.ts";
import { TabMotion } from "./tab-motion.ts";
import { anchorOf, groupOfLabel, labelId, type DropSide, type TabDrag } from "./tab-ui.ts";

/** How far past the row the pointer may go and still be dragging along it. */
const DRAG_LEAVE_PX = 16;

/** What the members of a row of tabs share: how a drag along it is held, and its motion. */
interface Row {
  readonly vertical: boolean;
  /** The handlers that press, drag and let go of one thing in the row. */
  handlers(source: string, press: (keyboard: boolean) => void): PointerHandlers;
  /** Notes that one of the row's tabs changed how it looks, so the mark follows. */
  restyled(): void;
  /** Whether the row's tabs show a bright copy of their labels for the moving mark. */
  readonly lit: boolean;
  /** Presses the mark down with the tab on show, until the pointer lets go. */
  pressMark(): void;
}

interface PointerHandlers {
  onPointerDown(event: ReactPointerEvent<HTMLElement>): void;
  onPointerMove(event: ReactPointerEvent<HTMLElement>): void;
  onPointerUp(event: ReactPointerEvent<HTMLElement>): void;
  onPointerCancel(event: ReactPointerEvent<HTMLElement>): void;
}

const RowContext = createContext<Row | null>(null);

function useRow(): Row {
  const row = useContext(RowContext);
  if (!row) throw new Error("a tab is rendered outside a row of tabs");
  return row;
}

/** The tabs of the row in order, each with its group, as the layout has them now. */
function rowOf(app: ShellApp): RowTab[] {
  const state = app.layout.getState();
  return selectTabOrder(state).map((tab) => ({ tab, group: state.tabs[tab]?.group ?? null }));
}

/**
 * Holds a drag along a row of tabs: what is pressed, where it would land, and letting
 * it go. `list` is the element the row's tabs and labels are in.
 */
function useRowDrag(
  app: ShellApp,
  list: RefObject<HTMLElement | null>,
  vertical: boolean,
  motion: RefObject<TabMotion | null>,
): Row["handlers"] {
  const held = useRef<{
    source: string;
    pointerId: number;
    x: number;
    y: number;
    moved: boolean;
  } | null>(null);
  const along = useCallback(
    (event: { clientX: number; clientY: number }) => (vertical ? event.clientY : event.clientX),
    [vertical],
  );

  // Escape puts what is being dragged back where it was.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || !held.current?.moved) return;
      held.current = null;
      app.tabUi.getState().setDrag(null);
      event.preventDefault();
      event.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [app]);

  const boxes = (): DropBox[] => {
    const element = list.current;
    if (!element) return [];
    return [...element.querySelectorAll<HTMLElement>(":scope > [data-tab-id]")].map((item) => {
      const box = item.getBoundingClientRect();
      return {
        id: item.dataset["tabId"] ?? "",
        group: item.dataset["group"] ?? null,
        start: vertical ? box.top : box.left,
        end: vertical ? box.bottom : box.right,
        folded: item.dataset["folded"] === "true",
      };
    });
  };

  const letGo = (source: string, drag: TabDrag | null) => {
    const ui = app.tabUi.getState();
    if (!drag?.target) {
      ui.setDrag(null);
      return;
    }
    const row = rowOf(app);
    const group = groupOfLabel(source);
    const place = { target: drag.target, side: drag.side };
    const intent = group ? groupDropIntent(row, group, place) : tabDropIntent(row, source, place);
    if (!intent) {
      ui.setDrag(null);
      return;
    }
    // The row keeps the drop's look until the layout follows, or for so long.
    ui.setDrag({ ...drag, dropped: true });
    const settle = window.setTimeout(() => {
      if (app.tabUi.getState().drag?.dropped) app.tabUi.getState().setDrag(null);
    }, DRAG_SETTLE_TIMEOUT_MS);
    const send = () =>
      app.client.layoutApply(intent).catch((error: unknown) => {
        window.clearTimeout(settle);
        app.tabUi.getState().setDrag(null);
        app.notices.push("error", error instanceof Error ? error.message : String(error));
      });
    // The last tab of a group that leaves it takes the group with it: its label and
    // line draw back first.
    const own = group ? null : (row.find((entry) => entry.tab === source)?.group ?? null);
    const last = own !== null && row.filter((entry) => entry.group === own).length === 1;
    if (intent.type === "tab_place" && last && intent.group !== own && motion.current) {
      void motion.current.groupExit(own).then(send);
    } else {
      void send();
    }
  };

  return (source, press) => ({
    onPointerDown: (event) => {
      if (event.button !== 0) return;
      if ((event.target as Element).closest(".tab-close, input")) return;
      held.current = {
        source,
        pointerId: event.pointerId,
        x: event.clientX,
        y: event.clientY,
        moved: false,
      };
    },
    onPointerMove: (event) => {
      const drag = held.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      const ui = app.tabUi.getState();
      if (!drag.moved) {
        const far = Math.max(Math.abs(event.clientX - drag.x), Math.abs(event.clientY - drag.y));
        if (far < DRAG_START_PX) return;
        drag.moved = true;
        event.currentTarget.setPointerCapture(event.pointerId);
        ui.close();
        ui.setDrag({ source, target: null, side: "before", dropped: false });
      }
      const now = ui.drag;
      if (!now || now.dropped) return;
      const box = list.current?.getBoundingClientRect();
      const across = vertical ? event.clientX : event.clientY;
      const outside =
        box !== undefined &&
        (vertical
          ? across < box.left - DRAG_LEAVE_PX || across > box.right + DRAG_LEAVE_PX
          : across < box.top - DRAG_LEAVE_PX || across > box.bottom + DRAG_LEAVE_PX);
      if (outside) {
        if (now.target !== null) ui.setDrag({ ...now, target: null });
        return;
      }
      const place = dropPlace(boxes(), source, along(event));
      if (!place) return;
      if (place.target !== now.target || place.side !== now.side) {
        ui.setDrag({ ...now, target: place.target, side: place.side });
      }
    },
    onPointerUp: (event) => {
      const drag = held.current;
      held.current = null;
      if (!drag || drag.pointerId !== event.pointerId) return;
      if (!drag.moved) {
        press(false);
        return;
      }
      letGo(source, app.tabUi.getState().drag);
    },
    onPointerCancel: () => {
      if (held.current?.moved) app.tabUi.getState().setDrag(null);
      held.current = null;
    },
  });
}

/**
 * Keeps a tab's label and what its panes have gathered up to date, without rendering,
 * and returns the pictures of its programs, which render when they change.
 */
function useTabMarks(
  app: ShellApp,
  tab: TabId,
  holder: RefObject<HTMLElement | null>,
  renaming: boolean,
) {
  const label = useRef<HTMLSpanElement>(null);
  const lit = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    // The label is another element once a rename is over.
    if (renaming) return undefined;
    let shown: string | undefined;
    return followTab(app, tab, (description) => {
      const element = label.current;
      if (element && description.label !== shown) {
        shown = description.label;
        // Set as text: a label comes from a program and is never markup.
        element.textContent = description.label;
        if (lit.current) lit.current.textContent = description.label;
        element.parentElement?.setAttribute("title", description.label);
      }
      const tabElement = holder.current;
      if (tabElement) mark(tabElement, "activity", description.activity, "idle");
    });
  }, [app, tab, holder, renaming]);
  const icons = useSyncExternalStore(
    useCallback((changed) => followTab(app, tab, changed), [app, tab]),
    () => describeTabNow(app, tab).icons.join(" "),
  );
  return { label, lit, icons: icons === "" ? [] : (icons.split(" ") as PaneIcon[]) };
}

/**
 * The tiles of a tab's programs: one, or several, each behind the one before it and a
 * little along, the one the tab is named after in front.
 */
function TabIcons({ icons }: { icons: readonly PaneIcon[] }) {
  if (icons.length <= 1) {
    return (
      <span className="tab-icon">
        <ProgramTile tile={icons[0] ?? "terminal"} place="tab" />
      </span>
    );
  }
  return (
    <span className="tab-icon tab-icons" data-count={icons.length}>
      {icons.map((icon, index) => (
        <span key={index} className="tab-icons-one">
          <ProgramTile tile={icon} place="tab" />
        </span>
      ))}
    </span>
  );
}

/** A tab being renamed: a field in the tab's place. */
function RenameField({ tab }: { tab: TabId }) {
  const app = useShell();
  const field = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    const element = field.current;
    if (!element) return;
    element.value = app.layout.getState().tabs[tab]?.title ?? "";
    element.focus();
    element.select();
  }, [app, tab]);
  const finish = (keep: boolean) => {
    if (done.current) return;
    done.current = true;
    const title = field.current?.value.trim() ?? "";
    if (keep) app.commands.run("lume.tab.rename", { tab, title: title === "" ? null : title });
    app.tabUi.getState().endRename();
    app.views.focus();
  };
  return (
    <input
      ref={field}
      className="tab-rename"
      aria-label="Tab name"
      spellCheck={false}
      onBlur={() => finish(true)}
      onKeyDown={(event) => {
        if (event.key === "Enter") finish(true);
        else if (event.key === "Escape") finish(false);
        // What is typed here is the name, not a key for the shell.
        event.stopPropagation();
      }}
    />
  );
}

/** The members of a row the arrow keys move the keyboard among: what is not folded away. */
const ROVING = ".tab:not([data-folded='true']) > [role=tab], .tab-group-label";

/** What a tab or a label of the row shows of a drag going on. */
interface DragLook {
  /** Which side of it what is dragged would land on. */
  readonly drop?: DropSide;
  /** Whether it is what is being dragged. */
  readonly dragging?: boolean;
  /** Whether what is dragged would join its group. */
  readonly joining?: boolean;
}

/**
 * One tab among its window's tabs: pressed to show it, dragged to another place among
 * them, renamed in place, given a colour, locked, put in a group. `vertical` says the
 * tabs are listed one under another, so a tab is dragged up and down, not sideways,
 * and the up and down arrows move the keyboard from one tab to the next.
 */
export function TabItem({
  tab,
  active,
  group = null,
  folded = false,
  last = false,
  look = NO_LOOK,
}: {
  tab: TabId;
  active: boolean;
  group?: Group | null;
  folded?: boolean;
  last?: boolean;
  look?: DragLook;
}) {
  const app = useShell();
  const row = useRow();
  const holder = useRef<HTMLButtonElement>(null);
  const entry = useStore(app.layout, (state) => state.tabs[tab]);
  const renaming = useStore(app.tabUi, (state) => state.renaming === tab);
  const { label, lit, icons } = useTabMarks(app, tab, holder, renaming);
  const closeHint = useKeyHint("lume.tab.close");
  const color = entry?.color ?? null;
  const locked = entry?.locked === true;
  const muted = entry?.muted === true;

  // A new colour, or a new group, is a new colour for the mark.
  useLayoutEffect(() => row.restyled(), [row, color, group?.color]);

  const show = () => {
    app.client
      .layoutApply({ type: "tab_activate", target: { type: "id", tab } })
      .catch(() => undefined);
  };
  const openMenu = (element: Element, keyboard: boolean) => {
    const ui = app.tabUi.getState();
    if (!active) show();
    ui.open({ kind: "tab", tab, anchor: anchorOf(element), keyboard });
  };
  const handlers = row.handlers(tab, () => {
    show();
    app.views.focus();
  });

  return (
    <div
      className="tab"
      data-tab={tab}
      data-tab-id={tab}
      data-active={active}
      data-color={color ?? undefined}
      data-group={group?.id}
      data-group-color={group?.color}
      data-group-last={(group !== null && last) || undefined}
      data-tinted={color !== null || group !== null}
      data-folded={folded || undefined}
      data-muted={muted || undefined}
      data-dragging={look.dragging || undefined}
      data-drop-side={look.drop}
      data-joining={look.joining || undefined}
      inert={folded || undefined}
      {...handlers}
      onPointerDown={(event) => {
        if (renaming) return;
        // The mark is pressed down with the tab it is on.
        if (event.button === 0 && active) row.pressMark();
        handlers.onPointerDown(event);
      }}
      onDoubleClick={() => app.tabUi.getState().startRename(tab)}
      onAuxClick={(event) => {
        // The middle button closes a tab, as it does wherever there are tabs.
        if (event.button === 1 && !locked) app.commands.run("lume.tab.close", { tab });
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        if (renaming) return;
        // A menu asked for from the keyboard comes with no place of the pointer's.
        openMenu(event.currentTarget, event.clientX === 0 && event.clientY === 0);
      }}
    >
      {renaming ? (
        <RenameField tab={tab} />
      ) : (
        <button
          ref={holder}
          type="button"
          role="tab"
          aria-selected={active}
          tabIndex={active ? 0 : -1}
          className="tab-label"
          onKeyDown={(event) => {
            if (event.key === "F2") {
              app.tabUi.getState().startRename(tab);
              return;
            }
            if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
              event.preventDefault();
              openMenu(event.currentTarget.parentElement ?? event.currentTarget, true);
              return;
            }
            // The arrows move the keyboard among the tabs without showing them: a
            // tab is shown when it is pressed, with Enter or the space bar.
            const tabs = event.currentTarget.closest("[role=tablist]");
            const way = row.vertical ? "vertical" : "horizontal";
            if (tabs && roveWithin(event, tabs, ROVING, way)) event.preventDefault();
          }}
          onClick={(event) => {
            // A press with the pointer is the tab's own business, above; this is the
            // press a key makes.
            if (event.detail === 0) show();
          }}
        >
          <TabIcons icons={icons} />
          <span className="tab-title" ref={label} />
          <span className="tab-activity" aria-hidden="true" />
          {row.lit && (
            <span className="tab-lit" aria-hidden="true">
              <TabIcons icons={icons} />
              <span className="tab-title" ref={lit} />
            </span>
          )}
        </button>
      )}
      {muted && (
        <span className="tab-muted" title="Muted: the tab does not ask for attention">
          <BellOffIcon />
        </span>
      )}
      {locked ? (
        <span className="tab-lock" title="Locked: the tab is not closed by closing tabs">
          <LockIcon />
        </span>
      ) : (
        <button
          type="button"
          className="tab-close control"
          aria-label="Close tab"
          title={`Close tab${closeHint}`}
          tabIndex={-1}
          onClick={() => app.commands.run("lume.tab.close", { tab })}
        >
          <CloseIcon size={11} />
        </button>
      )}
      {group && <span className="tab-group-line" aria-hidden="true" />}
    </div>
  );
}

const NO_LOOK: DragLook = {};

/**
 * The label in front of a group's tabs: its picture and name, and, while the group is
 * folded, how many tabs it holds and whether one of them is waiting for the user.
 */
export function GroupLabel({
  group,
  tabs,
  look = NO_LOOK,
}: {
  group: Group;
  tabs: readonly TabId[];
  look?: DragLook;
}) {
  const app = useShell();
  const row = useRow();
  const button = useRef<HTMLButtonElement>(null);
  const folded = group.collapsed === true;
  const name = group.name ?? UNNAMED_GROUP;
  const members = tabs.join(" ");

  // Whether a folded tab is waiting for the user, written without rendering.
  useLayoutEffect(() => {
    const element = button.current;
    if (!element) return undefined;
    const activities = new Map<TabId, Activity>();
    const stops = members
      .split(" ")
      .filter((tab) => tab !== "")
      .map((tab) =>
        followTab(app, tab, (description) => {
          activities.set(tab, description.activity);
          const most = greatest(activities.values());
          mark(element, "waiting", String(most.startsWith("attention")), "false");
        }),
      );
    return () => stops.forEach((stop) => stop());
  }, [app, members]);

  const toggle = () => app.commands.run("lume.group.toggle", { group: group.id });
  const edit = (focusName: boolean) => {
    const element = button.current;
    if (!element) return;
    app.tabUi
      .getState()
      .open({ kind: "group", group: group.id, anchor: anchorOf(element), focusName });
  };
  const handlers = row.handlers(labelId(group.id), () => toggle());

  return (
    <button
      ref={button}
      type="button"
      className="tab-group-label"
      tabIndex={-1}
      data-tab-id={labelId(group.id)}
      data-group={group.id}
      data-group-color={group.color}
      data-collapsed={folded || undefined}
      data-muted={group.muted || undefined}
      data-empty={
        (group.name === undefined && group.icon === undefined && !group.muted) || undefined
      }
      data-dragging={look.dragging || undefined}
      data-drop-side={look.drop}
      data-joining={look.joining || undefined}
      aria-expanded={!folded}
      aria-label={`${name} tab group, ${tabCount(tabs.length)}, ${folded ? "collapsed" : "expanded"}`}
      title={`${name} · ${tabCount(tabs.length)} (click to ${folded ? "expand" : "collapse"}, right-click to edit)`}
      {...handlers}
      onClick={(event) => {
        // The pointer's press is the drag's to tell from a drag; this is a key's.
        if (event.detail === 0) toggle();
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        edit(false);
      }}
      onKeyDown={(event) => {
        if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
          event.preventDefault();
          edit(false);
          return;
        }
        const list = event.currentTarget.closest("[role=tablist]");
        const way = row.vertical ? "vertical" : "horizontal";
        if (list && roveWithin(event, list, ROVING, way)) event.preventDefault();
      }}
    >
      {group.icon !== undefined && (
        <span className="tab-group-icon">
          <GroupGlyph icon={group.icon} />
        </span>
      )}
      {group.name !== undefined && <span className="tab-group-name">{group.name}</span>}
      {group.muted && (
        <span className="tab-group-muted" title="Muted: its tabs do not ask for attention">
          <BellOffIcon size={11} />
        </span>
      )}
      <span className="tab-group-count">
        <span>
          <span className="tab-group-waiting" title="A tab in this group is waiting for you" />
          <span className="tab-group-number">{tabs.length}</span>
        </span>
      </span>
      <span className="tab-group-line" aria-hidden="true" />
    </button>
  );
}

/** What a drag going on shows on each thing in the row. */
function dragLooks(drag: TabDrag | null, items: readonly StripItem[], app: ShellApp) {
  const looks = new Map<string, DragLook>();
  if (!drag) return looks;
  const dragged = groupOfLabel(drag.source);
  let joining: GroupId | null = null;
  if (drag.target !== null && drag.target !== drag.source && dragged === null) {
    joining = joinedGroup(rowOf(app), drag.source, drag.target, drag.side);
  }
  for (const item of items) {
    const id = item.kind === "group" ? labelId(item.group.id) : item.tab;
    const group = item.kind === "group" ? item.group.id : (item.group?.id ?? null);
    const look: { -readonly [Key in keyof DragLook]: DragLook[Key] } = {};
    if (id === drag.source || (dragged !== null && group === dragged)) look.dragging = true;
    if (drag.target === id && drag.target !== drag.source) look.drop = drag.side;
    if (joining !== null && group === joining) look.joining = true;
    looks.set(id, look);
  }
  return looks;
}

/** The things of a row, rendered: each group's label, then its tabs. */
function RowItems({
  items,
  active,
  drag,
}: {
  items: readonly StripItem[];
  active: TabId | null;
  drag: TabDrag | null;
}) {
  const app = useShell();
  const looks = useMemo(() => dragLooks(drag, items, app), [drag, items, app]);
  return items.map((item) =>
    item.kind === "group" ? (
      <GroupLabel
        key={labelId(item.group.id)}
        group={item.group}
        tabs={item.tabs}
        look={looks.get(labelId(item.group.id))}
      />
    ) : (
      <TabItem
        key={item.tab}
        tab={item.tab}
        active={item.tab === active}
        group={item.group}
        folded={item.folded}
        last={item.last}
        look={looks.get(item.tab)}
      />
    ),
  );
}

/** The row's things, as the layout has them now, rebuilt only when they change. */
function useStripItems(app: ShellApp): readonly StripItem[] {
  const key = useStore(app.layout, (state) => (state.ready ? selectStripKey(state) : ""));
  // The key says when the items change; they are read from the store as it is then.
  return useMemo(() => {
    const state = app.layout.getState();
    return state.ready && key !== "" ? stripItems(state) : NO_ITEMS;
  }, [app, key]);
}

/** Lets go of a drag that was let go, once the layout has followed it. */
function useSettledDrag(app: ShellApp, items: readonly StripItem[]): void {
  useEffect(() => {
    const ui = app.tabUi.getState();
    if (ui.drag?.dropped) ui.setDrag(null);
  }, [app, items]);
}

/** The row of tabs and the button that opens another. */
export function TabStrip() {
  const app = useShell();
  const items = useStripItems(app);
  const active = useStore(app.layout, (state) => (state.ready ? selectActiveTab(state) : null));
  const setting = useStore(
    app.config,
    (state) => state.view?.config.appearance["tab-highlight"] ?? "glow",
  );
  const motions = useStore(app.plugins, (state) => state.shown?.motions);
  // A plugin's style is made once for each time the plugins change, so that the row is
  // restyled only when the style is another.
  const style = useMemo(() => tabStyle(setting, motions), [setting, motions]);
  const drag = useStore(app.tabUi, (state) => state.drag);
  const renaming = useStore(app.tabUi, (state) => state.renaming);
  const list = useRef<HTMLDivElement>(null);
  const markRef = useRef<HTMLSpanElement>(null);
  const motion = useRef<TabMotion | null>(null);
  const lastRenaming = useRef<TabId | null>(null);
  const current = useRef({ style, active });
  current.current = { style, active };
  useSettledDrag(app, items);

  useLayoutEffect(() => {
    const element = list.current;
    const shape = markRef.current;
    if (!element || !shape) return undefined;
    const made = new TabMotion(element, shape);
    motion.current = made;
    const ui = app.tabUi.getState();
    ui.setRow({
      groupExit: (group) => made.groupExit(group),
      groupFade: (group) => made.groupFade(group),
    });
    // Tabs grow and shrink with their labels: the mark keeps to its tab, without moving.
    const resized = new ResizeObserver(() =>
      made.sync({ ...current.current, animate: false, renamed: null }),
    );
    resized.observe(element);
    return () => {
      resized.disconnect();
      made.dispose();
      motion.current = null;
      if (app.tabUi.getState().row !== null) app.tabUi.getState().setRow(null);
    };
  }, [app]);

  // After every render: the row's tabs slide to their places, and the mark to its tab.
  useLayoutEffect(() => {
    const made = motion.current;
    if (!made) return;
    const renamed =
      lastRenaming.current !== null && renaming === null ? lastRenaming.current : null;
    lastRenaming.current = renaming;
    made.sync({ style, active, animate: made.pointerRecent(), renamed });
  });

  // Folding a group or opening a gap moves tabs by the style sheet: the mark follows.
  const gap = drag ? `${drag.target ?? ""}:${drag.side}` : "";
  useEffect(() => motion.current?.followLayout(), [items, gap]);

  const rowHandlers = useRowDrag(app, list, false, motion);
  const handlers = useRef(rowHandlers);
  handlers.current = rowHandlers;
  const row = useMemo<Row>(
    () => ({
      vertical: false,
      lit: true,
      handlers: (source, press) => handlers.current(source, press),
      restyled: () => motion.current?.sync({ ...current.current, animate: false, renamed: null }),
      pressMark: () => motion.current?.press(),
    }),
    [],
  );

  return (
    <div
      className="tab-strip"
      data-drop={NEW_TAB_DROP}
      onPointerDownCapture={() => motion.current?.pointer()}
    >
      <div className="tab-list" role="tablist" aria-label="Tabs" ref={list}>
        <span className="tab-mark" ref={markRef} aria-hidden="true" />
        <RowContext value={row}>
          <RowItems items={items} active={active} drag={drag} />
        </RowContext>
      </div>
    </div>
  );
}

/**
 * The tabs as a column, for the panel that lists them down a side of the window. The
 * same tabs and labels as the row, dragged up and down; nothing travels between them.
 */
export function TabColumn() {
  const app = useShell();
  const items = useStripItems(app);
  const active = useStore(app.layout, (state) => (state.ready ? selectActiveTab(state) : null));
  const drag = useStore(app.tabUi, (state) => state.drag);
  const list = useRef<HTMLDivElement>(null);
  const motion = useRef<TabMotion | null>(null);
  useSettledDrag(app, items);
  const columnHandlers = useRowDrag(app, list, true, motion);
  const handlers = useRef(columnHandlers);
  handlers.current = columnHandlers;
  const row = useMemo<Row>(
    () => ({
      vertical: true,
      lit: false,
      handlers: (source, press) => handlers.current(source, press),
      restyled: () => undefined,
      pressMark: () => undefined,
    }),
    [],
  );
  return (
    <div
      className="tab-list"
      role="tablist"
      aria-label="Tabs"
      aria-orientation="vertical"
      ref={list}
    >
      <RowContext value={row}>
        <RowItems items={items} active={active} drag={drag} />
      </RowContext>
    </div>
  );
}

const NO_ITEMS: readonly StripItem[] = [];
