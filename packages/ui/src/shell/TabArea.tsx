/**
 * The middle of the window: the panes of the tab on show, and the dividers between
 * them.
 *
 * This renders one card for each pane that is on show and one handle for each gap
 * between the children of a split. It renders again only when the tab's structure
 * changes: another tab is shown, a pane is made or closed, a pane is zoomed. Where the
 * cards and handles lie is not its business.
 */

import { useLayoutEffect, useMemo, useRef } from "react";
import { useStore } from "zustand";

import { panesOf, selectActiveTab } from "../layout/selectors.ts";
import type { TreeNode } from "../layout/store.ts";
import type { Axis, SplitId } from "../layout/types.ts";
import { useShell } from "./context.ts";
import { useKeyHint } from "./hints.ts";
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

/** A gap between two panes, which the pointer can take hold of to resize them. */
function DividerHandle({ split, index, axis }: Gap) {
  const { area } = useShell();
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
      aria-orientation={axis === "row" ? "vertical" : "horizontal"}
      aria-label={axis === "row" ? "Resize columns" : "Resize rows"}
      title="Drag to resize; double-click to make equal"
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
  const tab = useStore(app.layout, (state) => (state.ready ? selectActiveTab(state) : null));
  const root = useStore(app.layout, (state) => (tab === null ? undefined : state.tabs[tab]?.root));
  const zoomed = useStore(app.layout, (state) =>
    tab === null ? null : (state.tabs[tab]?.zoomed ?? null),
  );
  const mode = useStore(
    app.config,
    (state) => state.view?.config.appearance["pane-header"] ?? "always",
  );

  const all = useMemo(() => (root ? panesOf(root) : []), [root]);
  const shown = zoomed !== null && all.includes(zoomed) ? [zoomed] : all;
  const gaps = useMemo(() => (root && zoomed === null ? gapsOf(root) : []), [root, zoomed]);
  // A header that is shown only when needed is left off a pane alone in its tab.
  const header = mode === "always" || (mode === "auto" && all.length > 1);

  return (
    <main ref={container} className="tab-area">
      {ready && tab === null && <EmptyWindow />}
      {shown.map((pane) => (
        <PaneCard key={pane} pane={pane} header={header} />
      ))}
      {gaps.map((gap) => (
        <DividerHandle key={`${gap.split}:${gap.index}`} {...gap} />
      ))}
    </main>
  );
}
