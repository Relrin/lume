/**
 * A plugin's list panel: the rows its code gives, drawn as a tree.
 *
 * Rows hold rows of their own, which open beneath them. Only the rows in sight are
 * made, so a panel of thousands scrolls as easily as one of ten. The panel takes the
 * keyboard as one stop: the up and down arrows move among the rows, the right arrow
 * opens a row or goes into it, the left arrow closes it or goes back to the row it is
 * in, Home and End go to the first and the last, and Enter does what the row's first
 * action does. A row's actions are buttons at its end, shown while the pointer is on
 * it or the keyboard is.
 *
 * Every action runs one of the plugin's own commands, given the panel's name, the
 * row's id and the ids of the rows it is in.
 */

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useStore } from "zustand";

import type { PanelDef, PanelRowData, PluginImage } from "../core/types.ts";
import { PluginIcon } from "../plugins/PluginIcon.tsx";
import { iconNamed } from "../plugins/store.ts";
import { useShell } from "./context.ts";
import { ChevronRightIcon } from "./icons.tsx";

/** How tall a row is, in pixels: the panel makes only the rows that fit. */
export const LIST_ROW_PX = 26;

/** How many rows past those in sight are made, above and below, so scrolling shows no gap. */
const OVERSCAN = 6;

/** How far in each level of rows stands from the one it is in. */
const INDENT_PX = 14;

/** A row as it is drawn: where it is in the tree, and whether it is open. */
export interface FlatRow {
  readonly row: PanelRowData;
  /** The ids of the rows it is in, and its own. */
  readonly path: readonly string[];
  readonly depth: number;
  readonly open: boolean;
  readonly parent: number | null;
}

/** What names a row in the tree: the ids from the top down to it. */
function keyOf(path: readonly string[]): string {
  return path.join("\u0000");
}

/** The rows in the order they are drawn, with those of closed rows left out. */
export function flatten(
  rows: readonly PanelRowData[],
  opened: ReadonlyMap<string, boolean>,
): FlatRow[] {
  const out: FlatRow[] = [];
  const walk = (level: readonly PanelRowData[], path: readonly string[], parent: number | null) => {
    for (const row of level) {
      const own = [...path, row.id];
      const children = row.children ?? [];
      const open = children.length > 0 && (opened.get(keyOf(own)) ?? row.expanded === true);
      const at = out.length;
      out.push({ row, path: own, depth: path.length, open, parent });
      if (open) walk(children, own, at);
    }
  };
  walk(rows, [], null);
  return out;
}

const NO_ROWS: readonly PanelRowData[] = [];

function Row({
  flat,
  index,
  active,
  images,
  plugin,
  id,
  choose,
  run,
  toggle,
}: {
  flat: FlatRow;
  index: number;
  active: boolean;
  images: Readonly<Record<string, PluginImage>> | undefined;
  plugin: string;
  id: string;
  choose: () => void;
  run: (action: number) => void;
  toggle: () => void;
}) {
  const { row, depth, open } = flat;
  const parent = (row.children ?? []).length > 0;
  return (
    <div
      id={id}
      className="list-row"
      role="treeitem"
      aria-level={depth + 1}
      aria-expanded={parent ? open : undefined}
      aria-selected={active}
      data-active={active || undefined}
      data-tone={row.tone ?? "neutral"}
      style={{ top: `${index * LIST_ROW_PX}px`, paddingLeft: `${6 + depth * INDENT_PX}px` }}
      onPointerDown={(event) => {
        if ((event.target as Element).closest("button")) return;
        choose();
      }}
      onDoubleClick={() => run(0)}
    >
      <span className="list-row-fold">
        {parent && (
          <button
            type="button"
            className="list-row-chevron"
            data-open={open || undefined}
            tabIndex={-1}
            aria-label={open ? "Close" : "Open"}
            onClick={toggle}
          >
            <ChevronRightIcon />
          </button>
        )}
      </span>
      {row.icon !== undefined && (
        <span className="list-row-icon">
          <PluginIcon icon={iconNamed(plugin, row.icon, images)} images={images} size={13} />
        </span>
      )}
      <span className="list-row-label">{row.label}</span>
      {row.detail !== undefined && <span className="list-row-detail">{row.detail}</span>}
      {row.badge !== undefined && <span className="list-row-badge">{row.badge}</span>}
      {(row.actions ?? []).length > 0 && (
        <span className="list-row-actions">
          {(row.actions ?? []).map((action, at) => (
            <button
              key={at}
              type="button"
              className="control list-row-action"
              tabIndex={-1}
              title={action.title}
              aria-label={action.title}
              onClick={() => run(at)}
            >
              {action.icon === undefined ? (
                <span className="list-row-action-text">{action.title}</span>
              ) : (
                <PluginIcon
                  icon={iconNamed(plugin, action.icon, images)}
                  images={images}
                  size={12}
                />
              )}
            </button>
          ))}
        </span>
      )}
    </div>
  );
}

/** A plugin's list panel. */
export function ListPanel({ panel }: { panel: PanelDef }) {
  const app = useShell();
  const rows = useStore(app.plugins, (state) => state.rows.get(panel.id) ?? NO_ROWS);
  const images = useStore(app.plugins, (state) => state.shown?.images);
  const problem = useStore(app.plugins, (state) => state.problems.get(panel.plugin));
  const [opened, setOpened] = useState<ReadonlyMap<string, boolean>>(new Map());
  const [active, setActive] = useState(0);
  const [scroll, setScroll] = useState({ top: 0, height: 0 });
  const box = useRef<HTMLDivElement>(null);

  // The panel is on screen while it is drawn.
  useEffect(() => app.visible.hold(panel.id), [app, panel.id]);

  const flat = useMemo(() => flatten(rows, opened), [rows, opened]);
  const empty = flat.length === 0;

  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return undefined;
    const measure = () => setScroll({ top: element.scrollTop, height: element.clientHeight });
    measure();
    const resized = new ResizeObserver(measure);
    resized.observe(element);
    return () => resized.disconnect();
  }, [empty]);
  const at = Math.min(active, Math.max(0, flat.length - 1));

  // The row with the keyboard stays in sight.
  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return;
    const top = at * LIST_ROW_PX;
    if (top < element.scrollTop) element.scrollTop = top;
    else if (top + LIST_ROW_PX > element.scrollTop + element.clientHeight) {
      element.scrollTop = top + LIST_ROW_PX - element.clientHeight;
    }
  }, [at]);

  const toggle = (index: number, open?: boolean) => {
    const target = flat[index];
    if (!target || (target.row.children ?? []).length === 0) return;
    const next = new Map(opened);
    next.set(keyOf(target.path), open ?? !target.open);
    setOpened(next);
  };

  const run = (index: number, action: number) => {
    const target = flat[index];
    const chosen = target?.row.actions?.[action];
    if (!target || !chosen) return;
    const own = typeof chosen.args === "object" && chosen.args !== null ? chosen.args : {};
    app.commands.run(chosen.command, {
      ...own,
      panel: panel.view,
      row: target.row.id,
      path: target.path,
    });
  };

  const first = Math.max(0, Math.floor(scroll.top / LIST_ROW_PX) - OVERSCAN);
  const last = Math.min(
    flat.length,
    Math.ceil((scroll.top + scroll.height) / LIST_ROW_PX) + OVERSCAN,
  );
  const rowId = (index: number) => `list-${panel.id}-${index}`;

  return (
    <div className="list-panel" data-problem={problem === undefined ? undefined : true}>
      {problem !== undefined && (
        <p className="list-panel-problem" role="status">
          This panel is held up: {problem}
        </p>
      )}
      {flat.length === 0 ? (
        <p className="list-panel-empty">Nothing to show yet</p>
      ) : (
        <div
          ref={box}
          className="list-panel-rows"
          role="tree"
          aria-label={panel.title}
          tabIndex={0}
          aria-activedescendant={rowId(at)}
          onScroll={(event) =>
            setScroll({
              top: event.currentTarget.scrollTop,
              height: event.currentTarget.clientHeight,
            })
          }
          onKeyDown={(event) => {
            const current = flat[at];
            let handled = true;
            switch (event.key) {
              case "ArrowDown":
                setActive(Math.min(flat.length - 1, at + 1));
                break;
              case "ArrowUp":
                setActive(Math.max(0, at - 1));
                break;
              case "Home":
                setActive(0);
                break;
              case "End":
                setActive(flat.length - 1);
                break;
              case "ArrowRight":
                if (current && (current.row.children ?? []).length > 0) {
                  if (current.open) setActive(at + 1);
                  else toggle(at, true);
                }
                break;
              case "ArrowLeft":
                if (current?.open) toggle(at, false);
                else if (current && current.parent !== null) setActive(current.parent);
                break;
              case "Enter":
                run(at, 0);
                break;
              default:
                handled = false;
            }
            if (handled) {
              event.preventDefault();
              event.stopPropagation();
            }
          }}
        >
          <div className="list-panel-space" style={{ height: `${flat.length * LIST_ROW_PX}px` }}>
            {flat.slice(first, last).map((row, offset) => {
              const index = first + offset;
              return (
                <Row
                  key={keyOf(row.path)}
                  flat={row}
                  index={index}
                  active={index === at}
                  images={images}
                  plugin={panel.plugin}
                  id={rowId(index)}
                  choose={() => setActive(index)}
                  run={(action) => run(index, action)}
                  toggle={() => toggle(index)}
                />
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
