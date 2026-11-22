/**
 * Every tab of the window as a list, under a button that stands at the end of the row
 * of tabs while the row holds more than fits. The list is searched by typing, says which
 * tabs are out of sight, and shows the one that is picked.
 */

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useStore } from "zustand";

import { selectActiveTab, selectTabOrder } from "../layout/selectors.ts";
import type { TabId } from "../layout/types.ts";
import type { ShellApp } from "./boot.ts";
import { useShell } from "./context.ts";
import { describeTabNow } from "./describing.ts";
import { ChevronDownIcon, ProgramTile, SearchIcon } from "./icons.tsx";
import { tabMatches } from "./tab-scroll.ts";

interface Listed {
  readonly tab: TabId;
  readonly label: string;
  readonly icon: ReturnType<typeof describeTabNow>["icons"][number] | undefined;
}

/** The window's tabs in order, each as it is called now. */
function listed(app: ShellApp): Listed[] {
  return selectTabOrder(app.layout.getState()).map((tab) => {
    const described = describeTabNow(app, tab);
    return { tab, label: described.label, icon: described.icons[0] };
  });
}

function TabListBox({
  app,
  away,
  close,
}: {
  app: ShellApp;
  /** The tabs that are out of sight in the row. */
  away: ReadonlySet<string>;
  close: (toTerminal: boolean) => void;
}) {
  const active = useStore(app.layout, (state) => (state.ready ? selectActiveTab(state) : null));
  const order = useStore(app.layout, (state) =>
    state.ready ? selectTabOrder(state).join(" ") : "",
  );
  // What the tabs are called is read as the list opens and as the tabs change.
  const all = useMemo(() => listed(app), [app, order]);
  const [query, setQuery] = useState("");
  const shown = useMemo(() => all.filter((one) => tabMatches(one.label, query)), [all, query]);
  const [picked, setPicked] = useState<TabId | null>(active);
  const at = shown.find((one) => one.tab === picked) ?? shown[0] ?? null;
  const field = useRef<HTMLInputElement>(null);
  const rows = useRef<HTMLDivElement>(null);

  useEffect(() => field.current?.focus({ preventScroll: true }), []);

  useLayoutEffect(() => {
    if (at === null) return;
    const row = rows.current?.querySelector<HTMLElement>(`[data-tab-row="${CSS.escape(at.tab)}"]`);
    if (typeof row?.scrollIntoView === "function") row.scrollIntoView({ block: "nearest" });
  }, [at?.tab]);

  const show = (tab: TabId) => {
    close(true);
    app.client
      .layoutApply({ type: "tab_activate", target: { type: "id", tab } })
      .catch(() => undefined);
  };

  return (
    <>
      <div className="tab-all-scrim" aria-hidden="true" onPointerDown={() => close(false)} />
      <div
        className="tab-all-list"
        role="dialog"
        aria-label="All tabs"
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            close(true);
            return;
          }
          if (event.key === "Enter") {
            event.preventDefault();
            if (at) show(at.tab);
            return;
          }
          if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
          event.preventDefault();
          if (shown.length === 0) return;
          const now = at ? shown.indexOf(at) : -1;
          const step = event.key === "ArrowDown" ? 1 : -1;
          const next = shown[(Math.max(now, 0) + step + shown.length) % shown.length];
          if (next) setPicked(next.tab);
        }}
      >
        <label className="tab-all-find">
          <SearchIcon size={13} />
          <input
            ref={field}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setPicked(null);
            }}
            placeholder="Find a tab"
            aria-label="Find a tab"
            role="combobox"
            aria-expanded="true"
            aria-controls="tab-all-rows"
            aria-activedescendant={at ? `tab-all-${at.tab}` : undefined}
            autoComplete="off"
            spellCheck={false}
          />
        </label>
        <div ref={rows} id="tab-all-rows" className="tab-all-rows" role="listbox" aria-label="Tabs">
          {shown.map((one) => (
            <button
              key={one.tab}
              id={`tab-all-${one.tab}`}
              type="button"
              className="tab-all-row"
              role="option"
              aria-selected={one.tab === at?.tab}
              tabIndex={-1}
              data-tab-row={one.tab}
              onPointerMove={() => {
                if (one.tab !== at?.tab) setPicked(one.tab);
              }}
              onClick={() => show(one.tab)}
            >
              <span className="tab-all-icon">
                <ProgramTile tile={one.icon ?? "terminal"} place="tab" />
              </span>
              <span className="tab-all-title">{one.label}</span>
              {one.tab === active ? (
                <small data-kind="current">current</small>
              ) : away.has(one.tab) ? (
                <small>out of view</small>
              ) : null}
            </button>
          ))}
          {shown.length === 0 && <p className="tab-all-empty">No tab matches “{query}”</p>}
        </div>
      </div>
    </>
  );
}

/**
 * The button that lists every tab, and its list. `away` is asked for as the list opens:
 * which tabs are out of sight just then.
 */
export function TabListButton({ away }: { away: () => string[] }) {
  const app = useShell();
  const count = useStore(app.layout, (state) => (state.ready ? selectTabOrder(state).length : 0));
  const [open, setOpen] = useState<ReadonlySet<string> | null>(null);
  return (
    <div className="tab-all">
      <button
        type="button"
        className="control tab-all-button"
        aria-expanded={open !== null}
        aria-haspopup="dialog"
        aria-label={`All ${count} tabs`}
        title={`All ${count} tabs`}
        onPointerDown={(event) => event.preventDefault()}
        onClick={() => setOpen(open === null ? new Set(away()) : null)}
      >
        <ChevronDownIcon size={12} />
        <span>{count}</span>
      </button>
      {open !== null && (
        <TabListBox
          app={app}
          away={open}
          close={(toTerminal) => {
            setOpen(null);
            if (toTerminal) app.views.focus();
          }}
        />
      )}
    </div>
  );
}
