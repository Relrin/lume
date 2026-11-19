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
 *
 * A row may say its rows are given only when it is opened. Opening it asks the plugin
 * for them; a line beneath it says they are on their way, and says so if none came, in
 * which case opening it again asks again.
 *
 * When the plugin shows other rows, the panel stays where it was: the row at its top
 * stays at its top and the row with the keyboard keeps it, wherever they are among
 * the new rows. A panel with no rows keeps its place too, so one that is emptied and
 * filled again does not jump back to its start.
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

/** How long a row waits for the rows it asked for before it says none came. */
export const EXPAND_WAIT_MS = 10_000;

/** How the asking for a row's rows stands. */
export type Asked = "waiting" | "failed";

/** A row as it is drawn: where it is in the tree, and whether it is open. */
export interface FlatRow {
  readonly row: PanelRowData;
  /** The ids of the rows it is in, and its own. */
  readonly path: readonly string[];
  readonly depth: number;
  readonly open: boolean;
  readonly parent: number | null;
  /**
   * Set on a line that is no row of the plugin's: it stands beneath an open row whose
   * rows have not come, and says how the asking for them stands.
   */
  readonly note?: Asked;
}

/** Whether a row's rows are still to be asked for. */
export function unloaded(row: PanelRowData): boolean {
  return row.lazy === true && row.loaded !== true;
}

/** Whether a row can be opened: it has rows, or will have when asked. */
function opens(row: PanelRowData): boolean {
  return (row.children ?? []).length > 0 || unloaded(row);
}

/** What names a row in the tree: the ids from the top down to it. */
function keyOf(path: readonly string[]): string {
  return path.join("\u0000");
}

/** What a line that stands for rows on their way is called among its siblings. */
const NOTE_ID = "\u0001";

/**
 * The rows in the order they are drawn, with those of closed rows left out. An open
 * row whose rows have not come has one line beneath it that says so; `asked` says how
 * the asking for them stands.
 */
export function flatten(
  rows: readonly PanelRowData[],
  opened: ReadonlyMap<string, boolean>,
  asked: ReadonlyMap<string, Asked> = new Map(),
): FlatRow[] {
  const out: FlatRow[] = [];
  const walk = (level: readonly PanelRowData[], path: readonly string[], parent: number | null) => {
    for (const row of level) {
      const own = [...path, row.id];
      const key = keyOf(own);
      const open = opens(row) && (opened.get(key) ?? row.expanded === true);
      const at = out.length;
      out.push({ row, path: own, depth: path.length, open, parent });
      if (!open) continue;
      if (unloaded(row)) {
        const note = asked.get(key) ?? "waiting";
        out.push({
          row: { id: NOTE_ID, label: note === "failed" ? "Could not load" : "Loading…" },
          path: [...own, NOTE_ID],
          depth: own.length,
          open: false,
          parent: at,
          note,
        });
      } else {
        walk(row.children ?? [], own, at);
      }
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
  const parent = opens(row);
  if (flat.note !== undefined) {
    return (
      <div
        id={id}
        className="list-row list-row-note"
        role="treeitem"
        aria-level={depth + 1}
        aria-selected={active}
        aria-busy={flat.note === "waiting" || undefined}
        data-active={active || undefined}
        data-note={flat.note}
        style={{ top: `${index * LIST_ROW_PX}px`, paddingLeft: `${6 + depth * INDENT_PX}px` }}
        onPointerDown={choose}
      >
        <span className="list-row-fold" />
        <span className="list-row-label">{row.label}</span>
      </div>
    );
  }
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
  const [asked, setAsked] = useState<ReadonlyMap<string, Asked>>(new Map());
  // The row with the keyboard: by what names it, and by where it was for when a row
  // of that name is gone.
  const [active, setActive] = useState<{ key: string | null; index: number }>({
    key: null,
    index: 0,
  });
  const [scroll, setScroll] = useState({ top: 0, height: 0 });
  const box = useRef<HTMLDivElement>(null);
  // The row at the top of what is in sight, and how far it is scrolled past.
  const anchor = useRef<{ key: string; past: number } | null>(null);
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  // The panel is on screen while it is drawn.
  useEffect(() => app.visible.hold(panel.id), [app, panel.id]);
  useEffect(() => {
    const waiting = timers.current;
    return () => {
      for (const timer of waiting.values()) clearTimeout(timer);
      waiting.clear();
    };
  }, []);

  const flat = useMemo(() => flatten(rows, opened, asked), [rows, opened, asked]);
  const at = useMemo(() => {
    const found =
      active.key === null ? -1 : flat.findIndex((row) => keyOf(row.path) === active.key);
    return found >= 0 ? found : Math.min(active.index, Math.max(0, flat.length - 1));
  }, [flat, active]);
  const choose = (index: number) => {
    const target = flat[index];
    setActive({ key: target ? keyOf(target.path) : null, index });
  };

  // Asks the plugin for the rows of a row, and says none came if none do in time.
  const ask = (path: readonly string[]) => {
    const key = keyOf(path);
    const mark = (state: Asked) => setAsked((now) => new Map(now).set(key, state));
    clearTimeout(timers.current.get(key));
    timers.current.set(
      key,
      setTimeout(() => mark("failed"), EXPAND_WAIT_MS),
    );
    mark("waiting");
    app.expandRow(panel.id, path).catch(() => {
      clearTimeout(timers.current.get(key));
      mark("failed");
    });
  };

  // An open row whose rows were never asked for is asked now: one opened here, or one
  // the plugin showed open. One whose rows came is no longer waited for.
  useEffect(() => {
    const pending = new Set<string>();
    for (const one of flat) {
      if (!one.open || one.note !== undefined || !unloaded(one.row)) continue;
      const key = keyOf(one.path);
      pending.add(key);
      if (!asked.has(key)) ask(one.path);
    }
    const done = [...asked.keys()].filter((key) => !pending.has(key));
    if (done.length === 0) return;
    for (const key of done) {
      clearTimeout(timers.current.get(key));
      timers.current.delete(key);
    }
    setAsked((now) => new Map([...now].filter(([key]) => pending.has(key))));
    // `ask` only reads what does not change between renders.
  }, [flat, asked]);

  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return undefined;
    const measure = () => setScroll({ top: element.scrollTop, height: element.clientHeight });
    measure();
    const resized = new ResizeObserver(measure);
    resized.observe(element);
    return () => resized.disconnect();
  }, []);

  // Other rows: the row that was at the top stays at the top, where there still is one.
  useLayoutEffect(() => {
    const element = box.current;
    const held = anchor.current;
    if (!element || !held) return;
    const index = flat.findIndex((row) => keyOf(row.path) === held.key);
    if (index < 0) return;
    const top = index * LIST_ROW_PX + held.past;
    if (Math.abs(element.scrollTop - top) >= 1) element.scrollTop = top;
  }, [flat]);

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

  const scrolled = (top: number, height: number) => {
    const first = flat[Math.floor(top / LIST_ROW_PX)];
    anchor.current = first ? { key: keyOf(first.path), past: top % LIST_ROW_PX } : null;
    setScroll({ top, height });
  };

  const toggle = (index: number, open?: boolean) => {
    const target = flat[index];
    if (!target || target.note !== undefined || !opens(target.row)) return;
    const key = keyOf(target.path);
    const next = open ?? !target.open;
    setOpened((now) => new Map(now).set(key, next));
    // Opened again after nothing came, a row is asked again.
    if (next && unloaded(target.row) && asked.get(key) === "failed") ask(target.path);
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
  const empty = flat.length === 0;

  return (
    <div className="list-panel" data-problem={problem === undefined ? undefined : true}>
      {problem !== undefined && (
        <p className="list-panel-problem" role="status">
          This panel is held up: {problem}
        </p>
      )}
      <div
        ref={box}
        className="list-panel-rows"
        role="tree"
        aria-label={panel.title}
        tabIndex={empty ? -1 : 0}
        aria-activedescendant={empty ? undefined : rowId(at)}
        onScroll={(event) =>
          scrolled(event.currentTarget.scrollTop, event.currentTarget.clientHeight)
        }
        onKeyDown={(event) => {
          const current = flat[at];
          let handled = true;
          switch (event.key) {
            case "ArrowDown":
              choose(Math.min(flat.length - 1, at + 1));
              break;
            case "ArrowUp":
              choose(Math.max(0, at - 1));
              break;
            case "Home":
              choose(0);
              break;
            case "End":
              choose(flat.length - 1);
              break;
            case "ArrowRight":
              if (current && current.note === undefined && opens(current.row)) {
                if (current.open) choose(at + 1);
                else toggle(at, true);
              }
              break;
            case "ArrowLeft":
              if (current?.open) toggle(at, false);
              else if (current && current.parent !== null) choose(current.parent);
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
        {empty ? (
          <p className="list-panel-empty">Nothing to show yet</p>
        ) : (
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
                  choose={() => choose(index)}
                  run={(action) => run(index, action)}
                  toggle={() => toggle(index)}
                />
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
