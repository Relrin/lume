/**
 * Draws status items: one small element for each, the same way for every item of a
 * kind.
 *
 * Everything an item carries is rendered as text or as numbers in a drawing. A bar's
 * length and a line's shape are attributes of the drawing, so nothing here needs a
 * style of its own; an item that names one of the theme's colours has it set as the
 * variable its drawing is coloured by.
 *
 * A plugin's item may be a button, which runs its command or opens its menu, and may
 * have a card shown while the pointer rests on it or the keyboard is on it. While a
 * plugin's item is drawn the window counts it as on screen, which the plugin's meters
 * are read for. An item of a plugin whose code was stopped looks held up, and says why.
 */

import {
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { useStore } from "zustand";

import type { PluginImage } from "../core/types.ts";
import type { PaneId } from "../layout/types.ts";
import { playSlot } from "../motion/slots.ts";
import { PluginIcon } from "../plugins/PluginIcon.tsx";
import { findNode } from "../plugins/store.ts";
import { AnchoredMenu } from "../shell/AnchoredMenu.tsx";
import type { ShellApp } from "../shell/boot.ts";
import { ShellContext } from "../shell/context.ts";
import {
  AlertIcon,
  BranchIcon,
  DotIcon,
  FolderIcon,
  PulseIcon,
  TerminalIcon,
} from "../shell/icons.tsx";
import { Menu } from "../shell/Menu.tsx";
import { menuRows } from "../shell/menus.ts";
import { anchorOf, type Anchor } from "../shell/tab-ui.ts";
import type { StatusStore } from "./registry.ts";
import {
  opensAbove,
  type MeterItem,
  type SparklineItem,
  type StatusCardRow,
  type StatusIconName,
  type StatusItem,
  type StatusSlot,
} from "./types.ts";

/** The size of a meter's bar, in the units it is drawn in. */
const METER = { width: 36, height: 4 } as const;

/** The size of a sparkline, in the units it is drawn in. */
const SPARKLINE = { width: 48, height: 14, inset: 1.5 } as const;

/** How long the pointer rests on an item before its card is shown. */
const CARD_DELAY_MS = 350;

/** How wide a card is, and how far it is kept from the window's edges. */
const CARD_WIDTH_PX = 240;
const CARD_EDGE_PX = 8;

/** How far from its item a card is. */
const CARD_GAP_PX = 6;

/** How heavy the line of a picture drawn as one of the top bar's tools is, as theirs. */
const TOOL_WEIGHT = 1.9;

/** How wide the menu an item opens is. */
const STATUS_MENU_WIDTH = 240;

function icon(name: StatusIconName): ReactNode {
  switch (name) {
    case "terminal":
      return <TerminalIcon />;
    case "branch":
      return <BranchIcon />;
    case "pulse":
      return <PulseIcon />;
    case "folder":
      return <FolderIcon />;
    case "dot":
      return <DotIcon />;
    case "alert":
      return <AlertIcon />;
  }
}

/** A share between nought and one, whatever was given. */
function share(value: number): number {
  return Number.isFinite(value) ? Math.min(Math.max(value, 0), 1) : 0;
}

function Meter({ item }: { item: MeterItem }) {
  const filled = share(item.value);
  return (
    <>
      <span className="status-label">{item.label}</span>
      <svg
        className="status-meter"
        width={METER.width}
        height={METER.height}
        viewBox={`0 0 ${METER.width} ${METER.height}`}
        role="meter"
        aria-label={item.label}
        aria-valuemin={0}
        aria-valuemax={1}
        aria-valuenow={filled}
        aria-valuetext={item.text}
      >
        <rect
          className="status-meter-track"
          width={METER.width}
          height={METER.height}
          rx={METER.height / 2}
        />
        <rect
          className="status-meter-fill"
          width={METER.width * filled}
          height={METER.height}
          rx={METER.height / 2}
        />
      </svg>
      {item.text !== undefined && <span className="status-value">{item.text}</span>}
    </>
  );
}

/** The points of a sparkline, spread over its width and between its own extremes. */
export function sparklinePoints(
  values: readonly number[],
  size: { readonly width: number; readonly height: number; readonly inset: number } = SPARKLINE,
): string {
  const points = values.filter((value) => Number.isFinite(value));
  if (points.length === 0) return "";
  const low = Math.min(...points);
  const high = Math.max(...points);
  const { width, height, inset } = size;
  const step = points.length > 1 ? (width - 2 * inset) / (points.length - 1) : 0;
  return points
    .map((value, index) => {
      // Values that are all alike run along the middle.
      const up = high === low ? 0.5 : (value - low) / (high - low);
      const x = inset + step * index;
      const y = height - inset - up * (height - 2 * inset);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

function Line({
  points,
  label,
  size = SPARKLINE,
}: {
  points: readonly number[];
  label: string;
  size?: { readonly width: number; readonly height: number; readonly inset: number };
}) {
  return (
    <svg
      className="status-sparkline"
      width={size.width}
      height={size.height}
      viewBox={`0 0 ${size.width} ${size.height}`}
      role="img"
      aria-label={label}
    >
      <polyline
        points={sparklinePoints(points, size)}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function Sparkline({ item }: { item: SparklineItem }) {
  return (
    <>
      <span className="status-label">{item.label}</span>
      <Line
        points={item.points}
        label={item.text === undefined ? item.label : `${item.label}: ${item.text}`}
      />
      {item.text !== undefined && <span className="status-value">{item.text}</span>}
    </>
  );
}

function body(item: StatusItem): ReactNode {
  switch (item.kind) {
    case "text":
      return <span className={item.mono ? "status-value" : "status-text"}>{item.text}</span>;
    case "icon":
      return (
        <>
          {item.picture === undefined && (
            <span className="status-icon" role="img" aria-label={item.label}>
              {icon(item.icon)}
            </span>
          )}
          {item.show_label && <span className="status-text">{item.label}</span>}
        </>
      );
    case "meter":
      return <Meter item={item} />;
    case "sparkline":
      return <Sparkline item={item} />;
    case "badge":
      return <span className="status-badge">{item.text}</span>;
    case "segmented":
      // Drawn by the item itself: its parts are buttons of their own.
      return null;
  }
}

/** The size of the line beside a row of a card. */
const CARD_LINE = { width: 64, height: 16, inset: 1.5 } as const;

/** Where what an item opens is drawn: in the shell the item is in, over everything else. */
function layerOf(element: Element | null): Element {
  return element?.closest(".shell") ?? document.body;
}

/**
 * The card shown while the pointer rests on an item: its rows, under the item, or over
 * it with `above`.
 */
function Card({
  rows,
  title,
  at,
  above,
  layer,
}: {
  rows: readonly StatusCardRow[];
  title: string;
  at: DOMRect;
  above: boolean;
  layer: Element;
}) {
  const left = Math.max(
    CARD_EDGE_PX,
    Math.min(at.left, window.innerWidth - CARD_WIDTH_PX - CARD_EDGE_PX),
  );
  // Over the item, it hangs from the item's top by its own height.
  const style = above
    ? {
        left: `${Math.round(left)}px`,
        top: `${Math.round(at.top - CARD_GAP_PX)}px`,
        transform: "translateY(-100%)",
      }
    : { left: `${Math.round(left)}px`, top: `${Math.round(at.bottom + CARD_GAP_PX)}px` };
  return createPortal(
    <div className="status-card" role="tooltip" data-above={above ? true : undefined} style={style}>
      {title !== "" && <div className="status-card-title">{title}</div>}
      {rows.map((row, index) => (
        <div key={index} className="status-card-row" data-tone={row.tone ?? "neutral"}>
          <span className="status-card-label">{row.label}</span>
          {row.points !== undefined && row.points.length > 1 && (
            <Line points={row.points} label={row.label} size={CARD_LINE} />
          )}
          <span className="status-card-value">{row.value}</span>
        </div>
      ))}
    </div>,
    layer,
  );
}

/** What changing means for an item: what it shows, written out. */
function shownOf(item: StatusItem): string {
  switch (item.kind) {
    case "text":
    case "badge":
      return item.text;
    case "meter":
      return `${item.text ?? ""}|${item.value}`;
    case "sparkline":
      return item.text ?? "";
    case "icon":
      return item.label;
    case "segmented":
      return item.selected ?? "";
  }
}

const NO_IMAGES: Readonly<Record<string, PluginImage>> = {};
const nothing = () => () => undefined;

/** The plugins' pictures, where the item is drawn in a window that has them. */
function useImages(app: ShellApp | null): Readonly<Record<string, PluginImage>> {
  const read = () => app?.plugins.getState().shown?.images ?? NO_IMAGES;
  return useSyncExternalStore(app ? app.plugins.subscribe : nothing, read, read);
}

/** The menu a plugin's item opens, under it. */
function StatusMenu({
  app,
  item,
  anchor,
  close,
  layer,
}: {
  app: ShellApp;
  item: StatusItem;
  anchor: Anchor;
  close: () => void;
  layer: Element;
}) {
  const shown = useStore(app.plugins, (state) => state.shown);
  const contexts = useStore(app.plugins, (state) => state.contexts);
  const known = useStore(app.commandStore, (state) => state.byId);
  const node = findNode(shown?.menus["menu.status"], item.menu ?? "");
  const items = menuRows(node?.children ?? [], {
    contexts,
    builtin: () => [],
    plugin: (child) => {
      const command = child.command;
      if (command === undefined || !known.has(command)) return undefined;
      return {
        key: child.key,
        label: child.title ?? known.get(command)?.title ?? command,
        ...(child.icon ? { icon: <PluginIcon icon={child.icon} images={shown?.images} /> } : {}),
        choose: () => app.commands.run(command, child.args ?? null),
      };
    },
  });
  if (items.length === 0) return null;
  // Presses in the menu are the menu's, not the top bar's, which would move the window.
  return createPortal(
    <div onMouseDown={(event) => event.stopPropagation()}>
      <AnchoredMenu
        anchor={anchor}
        width={STATUS_MENU_WIDTH}
        above={opensAbove(item.slot)}
        className="status-menu"
      >
        <Menu label={node?.title ?? "Menu"} items={items} close={close} />
      </AnchoredMenu>
    </div>,
    layer,
  );
}

/** One status item. */
export function StatusItemView({ item }: { item: StatusItem }) {
  const app = useContext(ShellContext);
  const images = useImages(app);
  const element = useRef<HTMLElement | null>(null);
  const [card, setCard] = useState<DOMRect | null>(null);
  const [menu, setMenu] = useState<Anchor | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  // A plugin's item is on screen while it is drawn: in a pane's header, as the item
  // its manifest declares, whichever pane it is for.
  const visibleId = item.plugin === undefined ? undefined : (item.id.split("@")[0] ?? item.id);
  useEffect(() => {
    if (!app || visibleId === undefined) return undefined;
    return app.visible.hold(visibleId);
  }, [app, visibleId]);

  // An item shows that what it shows changed, as the settings choose.
  const shown = shownOf(item);
  const first = useRef(true);
  useLayoutEffect(() => {
    if (first.current) {
      first.current = false;
      return undefined;
    }
    const animation = playSlot("status.change", element.current);
    return () => animation?.cancel();
  }, [shown]);

  useEffect(() => () => clearTimeout(timer.current), []);

  const rows = item.card ?? [];
  const showCard = () => {
    if (rows.length === 0 || menu !== null) return;
    clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      const at = element.current?.getBoundingClientRect();
      if (at) setCard(at);
    }, CARD_DELAY_MS);
  };
  const hideCard = () => {
    clearTimeout(timer.current);
    setCard(null);
  };

  const label = item.kind === "text" || item.kind === "badge" ? item.text : item.label;
  const tooltip =
    item.problem === undefined
      ? item.tooltip
      : `${label === "" ? "This item" : label} is held up: ${item.problem}`;
  const style =
    item.color === undefined
      ? undefined
      : ({ "--status-tone": `var(--lume-${item.color.replaceAll(".", "-")})` } as CSSProperties);
  // A picture in the top bar that is pressed is drawn as one of the bar's tools: the
  // picture alone on a square button, a badge's count over a corner of it.
  const toolLike =
    item.picture !== undefined &&
    item.slot.startsWith("topbar.") &&
    (item.kind === "badge" ||
      (item.kind === "icon" &&
        !item.show_label &&
        (item.command !== undefined || item.menu !== undefined)));
  const content = (
    <>
      {item.picture !== undefined && (
        <span className="status-picture">
          {toolLike ? (
            <PluginIcon icon={item.picture} images={images} size={16} weight={TOOL_WEIGHT} />
          ) : (
            <PluginIcon icon={item.picture} images={images} size={13} />
          )}
        </span>
      )}
      {body(item)}
      {item.problem !== undefined && (
        <span className="status-problem">
          <AlertIcon />
        </span>
      )}
    </>
  );
  const attributes = {
    "data-status": item.id,
    "data-kind": item.kind,
    "data-tool": toolLike || undefined,
    "data-tone": item.tone ?? "neutral",
    "data-problem": item.problem === undefined ? undefined : true,
    "data-separator": item.separator === true ? true : undefined,
    "data-plugin": item.plugin,
    title: card === null ? tooltip : undefined,
    style,
    onPointerEnter: showCard,
    onPointerLeave: hideCard,
  };

  const interactive = app !== null && (item.command !== undefined || item.menu !== undefined);
  if (item.kind === "segmented") {
    const { command } = item;
    return (
      <span
        ref={(node) => {
          element.current = node;
        }}
        role="group"
        aria-label={item.label === "" ? undefined : item.label}
        className="status-item status-segmented"
        {...attributes}
        title={undefined}
      >
        {item.choices.map((choice) => {
          const chosen = choice.value === item.selected;
          return (
            <button
              key={choice.value}
              type="button"
              className="status-segment"
              aria-pressed={chosen}
              aria-label={choice.label}
              title={choice.label}
              tabIndex={-1}
              disabled={app === null || command === undefined}
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => {
                // The chosen part is where things are already.
                if (chosen || app === null || command === undefined) return;
                const given =
                  typeof item.args === "object" && item.args !== null
                    ? (item.args as Record<string, unknown>)
                    : {};
                app.commands.run(command, { ...given, value: choice.value });
              }}
            >
              {choice.picture !== undefined && (
                <span className="status-picture">
                  <PluginIcon icon={choice.picture} images={images} size={11} />
                </span>
              )}
              <span className="status-segment-label">{choice.label}</span>
            </button>
          );
        })}
      </span>
    );
  }
  return (
    <>
      {interactive ? (
        <button
          ref={(node) => {
            element.current = node;
          }}
          type="button"
          className="status-item status-button"
          aria-haspopup={item.menu === undefined ? undefined : "menu"}
          aria-expanded={item.menu === undefined ? undefined : menu !== null}
          aria-label={item.kind === "icon" ? item.label : undefined}
          onFocus={showCard}
          onBlur={hideCard}
          onClick={(event) => {
            hideCard();
            if (item.menu !== undefined) {
              setMenu(menu === null ? anchorOf(event.currentTarget) : null);
              return;
            }
            if (item.command !== undefined) app.commands.run(item.command, item.args ?? null);
          }}
          {...attributes}
        >
          {content}
        </button>
      ) : (
        <span
          ref={(node) => {
            element.current = node;
          }}
          className="status-item"
          {...attributes}
        >
          {content}
        </span>
      )}
      {card !== null && rows.length > 0 && (
        <Card
          rows={rows}
          title={label}
          at={card}
          above={opensAbove(item.slot)}
          layer={layerOf(element.current)}
        />
      )}
      {app !== null && menu !== null && (
        <StatusMenu
          app={app}
          item={item}
          anchor={menu}
          layer={layerOf(element.current)}
          close={() => {
            setMenu(null);
            app.views.focus();
          }}
        />
      )}
    </>
  );
}

/**
 * Some status items of one slot, in the order given. With `pane`, only the items that
 * name that pane: a pane's header shows its own.
 */
export function StatusList({
  items,
  slot,
  pane,
}: {
  items: readonly StatusItem[];
  slot: StatusSlot;
  pane?: PaneId;
}) {
  const shown = pane === undefined ? items : items.filter((item) => item.pane === pane);
  if (shown.length === 0) return null;
  return (
    <span className="status-slot" data-slot={slot}>
      {shown.map((item) => (
        <StatusItemView key={item.id} item={item} />
      ))}
    </span>
  );
}

/** The status items a registry holds for one slot, kept up to date. */
export function StatusSlotItems({
  store,
  slot,
  pane,
}: {
  store: StatusStore;
  slot: StatusSlot;
  pane?: PaneId;
}) {
  const items = useStore(store, (state) => state.bySlot[slot]);
  return <StatusList items={items} slot={slot} pane={pane} />;
}
