/**
 * The middle of the window: the panes of the tab on show, and the dividers between
 * them.
 *
 * This renders one card for each pane that is on show and one handle for each gap
 * between the children of a split. It renders again only when the tab's structure
 * changes: another tab is shown, a pane is made or closed, a pane is zoomed. Where the
 * cards and handles lie is not its business.
 *
 * The panes that float over the tabs are rendered inside the same element, after the
 * others, so that they share the space the tab's panes are laid out in.
 */

import { useLayoutEffect, useMemo, useRef } from "react";
import { useStore } from "zustand";

import { panesOf, selectActiveTab, selectFloating, selectMinimised } from "../layout/selectors.ts";
import type { TreeNode } from "../layout/store.ts";
import type { Axis, SplitId } from "../layout/types.ts";
import { useShell } from "./context.ts";
import { FloatingLayer } from "./FloatingLayer.tsx";
import { useKeyHint } from "./hints.ts";
import { MinimisedTray } from "./MinimisedTray.tsx";
import { PaneCard } from "./PaneCard.tsx";
import { PlusIcon } from "./icons.tsx";

/** One gap between two children of a split. */
interface Gap {
  readonly split: SplitId;
  readonly index: number;
  readonly axis: Axis;
}

/** The gaps of a tree: one for each pair of neighbouring children of each split. */
function gapsOf(node: TreeNode, found: Gap[] = []): Gap[] {
  if (node.type === "pane") return found;
  for (let index = 0; index + 1 < node.children.length; index++) {
    found.push({ split: node.id, index, axis: node.axis });
  }
  for (const child of node.children) gapsOf(child, found);
  return found;
}

/**
 * A gap between two panes, which the pointer can take hold of to resize them and the
 * arrow keys can move.
 */
function DividerHandle({ split, index, axis }: Gap) {
  const { area, views } = useShell();
  const handle = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = handle.current;
    return element ? area.registerDivider(split, index, element) : undefined;
  }, [area, split, index]);
  return (
    <div
      ref={handle}
      className="divider"
      data-axis={axis}
      role="separator"
      // Reached with the Tab key, and then moved with the arrow keys. Where it stands
      // is written to it by what places it.
      tabIndex={0}
      aria-orientation={axis === "row" ? "vertical" : "horizontal"}
      aria-label={axis === "row" ? "Resize columns" : "Resize rows"}
      title="Drag to resize; double-click to make equal"
      onKeyDown={(event) => {
        // Escape gives the keyboard back to the terminal, as it does in the bar.
        if (event.key === "Escape") views.focus();
      }}
    />
  );
}

/** What a window with no tabs shows. */
function EmptyWindow() {
  const { commands } = useShell();
  const hint = useKeyHint("lume.tab.new");
  return (
    <div className="empty-window">
      <p>This window has no tabs.</p>
      <button type="button" className="empty-action" onClick={() => commands.run("lume.tab.new")}>
        <PlusIcon />
        New tab{hint}
      </button>
    </div>
  );
}

/** The panes of the tab on show. */
export function TabArea() {
  const app = useShell();
  const container = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const element = container.current;
    return element ? app.area.attach(element) : undefined;
  }, [app]);

  const ready = useStore(app.layout, (state) => state.ready);
  // A card belongs to one layout: another workspace's pane of the same name gets a
  // card of its own, and with it a terminal of its own.
  const epoch = useStore(app.layout, (state) => state.epoch);
  const tab = useStore(app.layout, (state) => (state.ready ? selectActiveTab(state) : null));
  const root = useStore(app.layout, (state) => (tab === null ? undefined : state.tabs[tab]?.root));
  const zoomed = useStore(app.layout, (state) =>
    tab === null ? null : (state.tabs[tab]?.zoomed ?? null),
  );
  // Panes that are put away do not fill the window: only those on show do.
  const floats = useStore(
    app.layout,
    (state) => state.ready && selectFloating(state).length > selectMinimised(state).length,
  );
  const mode = useStore(
    app.config,
    (state) => state.view?.config.appearance["pane-header"] ?? "always",
  );

  const all = useMemo(() => (root ? panesOf(root) : []), [root]);
  const shown = zoomed !== null && all.includes(zoomed) ? [zoomed] : all;
  // The panes of the window's tabs that show a plugin's view, on show or not, always
  // in the same order: a frame that is moved among its siblings starts again, so
  // their cards are never rendered in another order, nor taken out while their tabs
  // are merely out of sight.
  const views = useStore(app.layout, (state) =>
    state.ready
      ? Object.values(state.tabs)
          .flatMap((one) => panesOf(one.root))
          .filter((pane) => state.panes[pane]?.content.type === "plugin_view")
          .sort()
          .join(" ")
      : "",
  );
  const viewPanes = useMemo(() => (views === "" ? [] : views.split(" ")), [views]);
  const gaps = useMemo(() => (root && zoomed === null ? gapsOf(root) : []), [root, zoomed]);
  // A header that is shown only when needed is left off a pane alone in its tab.
  const header = mode === "always" || (mode === "auto" && all.length > 1);

  return (
    <main ref={container} className="tab-area">
      {ready && tab === null && !floats && <EmptyWindow />}
      {/*
       * The handles come before the cards, so that the Tab key reaches them from the
       * bar above: a terminal keeps the Tab key for its program, and nothing that
       * follows a card can be tabbed to from it. Where each lies is not decided here.
       */}
      {gaps.map((gap) => (
        <DividerHandle key={`${gap.split}:${gap.index}`} {...gap} />
      ))}
      {shown
        .filter((pane) => !viewPanes.includes(pane))
        .map((pane) => (
          <PaneCard key={`${epoch}:${pane}`} pane={pane} header={header} />
        ))}
      {viewPanes.map((pane) => (
        <PaneCard
          key={`${epoch}:${pane}`}
          pane={pane}
          header={header}
          parked={!shown.includes(pane)}
        />
      ))}
      <FloatingLayer />
      <MinimisedTray />
    </main>
  );
}
