/**
 * The status bar along the bottom of the window.
 *
 * It holds status items at its two ends, and while a server's layout is on show it says
 * at its left whose. It is shown only while it has something, so a window whose items
 * are all in the top bar, or that has none, gives its room to the panes. What opens from an item in it, a menu or a card, opens above the item.
 *
 * While the user's files have something wrong in them, the bar says so at its left
 * until they are mended, and lists what is wrong with the way to each file.
 *
 * A press on the bar that is not on a control leaves the keyboard with the terminal,
 * and Escape gives it back from an item that has it.
 */

import { useState } from "react";
import { useStore } from "zustand";

import { describeProblem } from "../config/apply.ts";
import type { ConfigFile } from "../core/client.ts";
import type { ConfigProblem } from "../core/types.ts";

import { selectActivePanes, selectFloating } from "../layout/selectors.ts";
import { LinkDot } from "../rex/LinkDot.tsx";
import { hostOnShow } from "../rex/model.ts";
import { StatusList } from "../status/StatusItem.tsx";
import { AnchoredMenu } from "./AnchoredMenu.tsx";
import { notices } from "./boot/index.ts";
import { reason } from "./boot/reason.ts";
import { useShell } from "./context.ts";
import { selectorLabel } from "./host-selector.ts";
import { MenuGlyph } from "./icons.tsx";
import { Menu, type MenuItem } from "./Menu.tsx";
import { sentence } from "./notices.ts";
import { anchorOf, type Anchor } from "./tab-ui.ts";

/** The parts of the bar a press is meant for. */
const CONTROLS = "button, [role=menu]";

/** The server whose layout is on show, its session, and how many panes of it are. */
function ServedStatus({ backend }: { backend: string }) {
  const app = useShell();
  const name = useStore(app.layout, (state) => state.name);
  const panes = useStore(
    app.layout,
    (state) => selectActivePanes(state).length + selectFloating(state).length,
  );
  const server = useStore(app.rex, (state) => hostOnShow(state.list, backend));
  const label = selectorLabel(name, backend, server, true);
  return (
    <span className="status-served">
      <LinkDot link={label.link} className="switcher-dot" />
      <span className="status-served-text">
        {label.host} · session {name} · {panes === 1 ? "1 pane" : `${panes} panes`}
        {label.link === "down" ? ` · ${label.said.toLowerCase()}` : ""}
      </span>
    </span>
  );
}

/** How wide the list of problems is. */
const PROBLEMS_WIDTH = 420;

/** The most problems the list shows; the files say the rest. */
const PROBLEMS_SHOWN = 12;

const NO_PROBLEMS: readonly ConfigProblem[] = [];

/** The file a problem is in, as the core opens it for editing, if it can. */
export function problemFile(problem: ConfigProblem): ConfigFile | null {
  if (problem.file === null) return null;
  const parts = problem.file.split(/[\\/]/);
  const name = parts.at(-1) ?? "";
  const folder = parts.at(-2) ?? "";
  const stem = name.replace(/\.toml$/i, "");
  if (folder === "layouts") return `layout:${stem}`;
  if (folder === "themes") return `theme:${stem}`;
  if (folder === "workspaces") return "workspace";
  if (name === "config.toml") return "config";
  return null;
}

/** What is wrong in the user's files, while something is: a button, and its list. */
function ConfigProblems() {
  const app = useShell();
  const failed = useStore(app.config, (state) => state.failed);
  const passed = useStore(app.config, (state) => state.view?.problems ?? NO_PROBLEMS);
  const [anchor, setAnchor] = useState<Anchor | null>(null);
  const broken = failed.length > 0;
  const problems = broken ? failed : passed;
  if (problems.length === 0) return null;

  const count = problems.length === 1 ? "1 problem" : `${problems.length} problems`;
  const items: MenuItem[] = [
    {
      key: "explained",
      kind: "note",
      label: broken
        ? "The files were not applied, and the configuration before them is still in use. Lume reads them again as soon as they are saved."
        : "These were passed over; everything else in the files is in use.",
      choose: () => undefined,
    },
    ...problems.slice(0, PROBLEMS_SHOWN).map((problem, index): MenuItem => {
      const file = problemFile(problem);
      return {
        key: `problem:${index}`,
        label: describeProblem(problem),
        separated: index === 0,
        ...(file === null ? { disabled: true } : { hint: "Open" }),
        choose: () => {
          if (file === null) return;
          app.client.configOpen(file).catch((error: unknown) => {
            notices.push("error", sentence(reason(error)));
          });
        },
      };
    }),
  ];
  return (
    <>
      <button
        type="button"
        className="status-problems control"
        data-level={broken ? "error" : "warning"}
        aria-haspopup="menu"
        aria-expanded={anchor !== null}
        onClick={(event) => setAnchor(anchor ? null : anchorOf(event.currentTarget))}
      >
        <MenuGlyph name="triangle-exclamation-point" size={13} />
        {broken ? `Config not applied · ${count}` : `Config · ${count}`}
      </button>
      {anchor && (
        <AnchoredMenu anchor={anchor} width={PROBLEMS_WIDTH} above>
          <Menu
            label="Problems in the configuration"
            items={items}
            close={() => {
              setAnchor(null);
              app.views.focus();
            }}
            keyboard
          />
        </AnchoredMenu>
      )}
    </>
  );
}

/** The status bar, when it has something to show. */
export function StatusBar() {
  const app = useShell();
  const left = useStore(app.status, (state) => state.bySlot["statusbar.left"]);
  const right = useStore(app.status, (state) => state.bySlot["statusbar.right"]);
  const backend = useStore(app.layout, (state) => state.backend);
  const troubled = useStore(
    app.config,
    (state) => state.failed.length > 0 || (state.view?.problems.length ?? 0) > 0,
  );
  if (left.length === 0 && right.length === 0 && backend === null && !troubled) return null;
  return (
    <footer
      className="status-bar"
      onKeyDown={(event) => {
        if (event.key !== "Escape" || event.defaultPrevented) return;
        if ((event.target as Element).closest("[role=menu]")) return;
        event.preventDefault();
        app.views.focus();
      }}
      onMouseDown={(event) => {
        if (event.button !== 0 || (event.target as Element).closest(CONTROLS)) return;
        event.preventDefault();
      }}
    >
      {backend !== null && <ServedStatus backend={backend} />}
      <ConfigProblems />
      <StatusList items={left} slot="statusbar.left" />
      <span className="status-bar-gap" />
      <StatusList items={right} slot="statusbar.right" />
    </footer>
  );
}
