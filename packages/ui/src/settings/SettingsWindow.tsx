/**
 * The settings window: a list of pages at the left, and the page on show beside it.
 *
 * A page is a few headings, each over a box of rows. Typing into the field above the
 * list shows the rows of every page that match, each under where it is found;
 * pressing one opens its page and marks the row for a moment.
 *
 * The window is drawn from the configuration in use and from the schema that says how
 * to show it, so a setting changed here, in a terminal window or by hand in the file
 * looks the same a moment later.
 */

import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { useStore } from "zustand";

import type { KeyBinding } from "../core/types.ts";
import { chordOf } from "../keys/chord.ts";
import {
  AppearanceIcon,
  CollapseIcon,
  GeneralIcon,
  SearchIcon,
  TerminalIcon,
} from "../shell/icons.tsx";
import { Notices } from "../shell/Notices.tsx";
import { sentence } from "../shell/notices.ts";
import { roveWithin } from "../shell/roving.ts";
import { WindowButtons } from "../shell/WindowControls.tsx";
import { currentPlatform, KeyLayout } from "../terminal/keyboard.ts";
import { TRAFFIC_LIGHTS_INSET_PX } from "../tuning.ts";
import type { SettingsApp } from "./boot.ts";
import {
  searchSettings,
  settingsPages,
  type SettingRow,
  type SettingsPage,
  type SettingWrite,
} from "./model.ts";
import { allPages, navOf } from "./registry.ts";
import { Button, Row } from "./rows.tsx";

/** The command that opens this window, and so, pressed in it, closes it. */
const OPEN_COMMAND = "lume.settings.open";

/** The parts of the title bar a press is meant for, and so does not move the window. */
const BAR_CONTROLS = "button, input, select, a";

/** The fields a key is typed into: Escape there is the field's, not the window's. */
const FIELDS = "input, select, textarea";

/** The drawing for a page's name in the list of pages. */
function PageIcon({ name }: { name: string }) {
  switch (name) {
    case "general":
      return <GeneralIcon />;
    case "appearance":
      return <AppearanceIcon />;
    case "terminal":
      return <TerminalIcon size={15} />;
    default:
      return <CollapseIcon size={15} />;
  }
}

/** The chords that close the window: the ones bound to the command that opens it. */
function closingChords(bindings: readonly KeyBinding[]): ReadonlySet<string> {
  const chords = new Set<string>();
  for (const binding of bindings) {
    const only = binding.keys.length === 1 ? binding.keys[0] : undefined;
    if (binding.command === OPEN_COMMAND && only !== undefined) chords.add(only);
  }
  return chords;
}

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** The title bar of a window whose bar the page draws: its name and its buttons. */
function TitleBar({ app, mac }: { app: SettingsApp; mac: boolean }) {
  const bar = useRef<HTMLElement>(null);
  const maximised = useStore(app.windowState, (state) => state.maximised);
  // The system's own window buttons lie over the left end of the bar on a Mac.
  useLayoutEffect(() => {
    const element = bar.current;
    if (element) element.style.paddingLeft = mac ? `${TRAFFIC_LIGHTS_INSET_PX}px` : "";
  }, [mac]);
  const ask = (asked: Promise<void>) => {
    asked.catch((error: unknown) => app.notices.push("error", sentence(reason(error))));
  };
  return (
    <header
      ref={bar}
      className="settings-titlebar"
      onMouseDown={(event) => {
        if (event.button !== 0 || (event.target as Element).closest(BAR_CONTROLS)) return;
        event.preventDefault();
        // A press moves the window; a double press makes it fill the screen or puts
        // it back.
        ask(event.detail === 2 ? app.client.windowControl("maximise") : app.client.windowDrag());
      }}
    >
      <span className="settings-titlebar-name">Settings</span>
      {!mac && (
        <WindowButtons
          maximised={maximised}
          ask={(action) => ask(app.client.windowControl(action))}
        />
      )}
    </header>
  );
}

/** A heading over a box of rows. */
function Group({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section className="settings-group">
      <h2>{label}</h2>
      <div className="settings-rows">{children}</div>
    </section>
  );
}

/** The whole of the settings window. `app` is what was started for the page. */
export function SettingsWindow({ app }: { app: SettingsApp }) {
  const view = useStore(app.config, (state) => state.view);
  const lists = useStore(app.lists);
  const asked = useStore(app.shown);
  const registered = useSyncExternalStore(
    app.registry.subscribe,
    app.registry.current,
    app.registry.current,
  );

  const pages = useMemo(
    () => (view ? allPages(settingsPages(app.schema, view, lists), registered) : []),
    [app.schema, view, lists, registered],
  );
  const nav = useMemo(() => navOf(pages), [pages]);

  const [picked, setPicked] = useState<string | null>(asked.page);
  const [query, setQuery] = useState("");
  const [flash, setFlash] = useState<string | null>(null);
  /** Why the last change asked for of a setting was not made, by the setting's path. */
  const [failures, setFailures] = useState<Readonly<Record<string, string>>>({});

  // Asked for again from outside, the window shows the page that was asked for.
  useEffect(() => {
    if (asked.asked === 0) return;
    setPicked(asked.page);
    setQuery("");
  }, [asked.asked, asked.page]);

  const current: SettingsPage | undefined = pages.find((page) => page.id === picked) ?? pages[0];
  const searching = query.trim() !== "";
  const found = useMemo(() => searchSettings(pages, query), [pages, query]);

  const search = useRef<HTMLInputElement>(null);
  const head = useRef<HTMLDivElement>(null);
  const scroller = useRef<HTMLDivElement>(null);

  const go = (page: string, row: string | null = null) => {
    setPicked(page);
    setQuery("");
    setFlash(row);
    const element = scroller.current;
    if (element && row === null) element.scrollTop = 0;
  };

  const write = (change: SettingWrite) => {
    app.write(change).then(
      () =>
        setFailures((held) =>
          change.path in held
            ? Object.fromEntries(Object.entries(held).filter(([path]) => path !== change.path))
            : held,
        ),
      (error: unknown) =>
        setFailures((held) => ({ ...held, [change.path]: sentence(reason(error)) })),
    );
  };

  // Escape empties the search field, and with nothing to empty closes the window; so
  // does the key that opened it.
  const closing = useMemo(() => closingChords(view?.keymap.bindings ?? []), [view]);
  const state = useRef({ query, closing });
  state.current = { query, closing };
  useEffect(() => {
    const mapping = { platform: currentPlatform(), layout: new KeyLayout() };
    const onKey = (event: KeyboardEvent) => {
      const chord = chordOf(event, mapping);
      if (chord !== null && state.current.closing.has(chord)) {
        event.preventDefault();
        app.close();
        return;
      }
      if (event.key !== "Escape" || event.defaultPrevented) return;
      const where = event.target instanceof Element ? event.target : null;
      if (state.current.query !== "") {
        event.preventDefault();
        setQuery("");
        return;
      }
      // A field keeps its own Escape: a list of choices that is open is only shut.
      if (where?.closest(FIELDS) && where !== search.current) return;
      event.preventDefault();
      app.close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [app]);

  // The window is named after what it is, for a window whose bar the system draws.
  useEffect(() => {
    document.title = "Settings";
  }, []);

  const rowOf = (row: SettingRow, open?: () => void) => (
    <Row
      key={row.path}
      row={row}
      write={write}
      failure={failures[row.path]}
      flash={flash === row.path}
      flashed={() => setFlash((held) => (held === row.path ? null : held))}
      open={open}
    />
  );

  const custom = view?.title_bar === "custom";
  const mac = view?.platform === "macos";
  const title = searching ? "Search" : (current?.label ?? "Settings");
  const meta = searching
    ? `${found.count} ${found.count === 1 ? "match" : "matches"}`
    : (current?.meta ?? "");
  const Preview = searching ? undefined : current?.preview;

  return (
    <div className="settings-window">
      {custom && <TitleBar app={app} mac={mac} />}
      <div className="settings-body">
        <aside className="settings-side">
          <label className="settings-search">
            <SearchIcon />
            <input
              ref={search}
              type="text"
              aria-label="Search settings"
              placeholder="Search settings"
              autoComplete="off"
              spellCheck={false}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>
          <nav
            className="settings-nav"
            aria-label="Settings sections"
            onKeyDown={(event) => {
              if (roveWithin(event, event.currentTarget, "button", "vertical")) {
                event.preventDefault();
              }
            }}
          >
            {nav.map((entry) =>
              entry.kind === "heading" ? (
                <h4 key={`heading:${entry.label}`}>{entry.label}</h4>
              ) : (
                <button
                  key={entry.page.id}
                  type="button"
                  className="settings-nav-item"
                  aria-current={!searching && entry.page.id === current?.id ? "page" : undefined}
                  // The list is one stop for the Tab key; the arrows move within it.
                  tabIndex={entry.page.id === current?.id ? 0 : -1}
                  onClick={() => go(entry.page.id)}
                >
                  <PageIcon name={entry.page.icon} />
                  <span className="settings-nav-label">{entry.page.label}</span>
                  {entry.page.meta !== undefined && (
                    <span className="settings-nav-meta">{entry.page.meta}</span>
                  )}
                </button>
              ),
            )}
          </nav>
        </aside>
        <main className="settings-main">
          <div ref={head} className="settings-head">
            <h1>{title}</h1>
            <span className="settings-meta">{meta}</span>
            <span className="settings-head-space" />
            <Button quiet onPress={app.openFile}>
              Open config.toml
            </Button>
          </div>
          <div
            ref={scroller}
            className="settings-scroll"
            onScroll={(event) => {
              // Written straight to the element: scrolling renders nothing.
              const scrolled = String(event.currentTarget.scrollTop > 2);
              const element = head.current;
              if (element && element.dataset["scrolled"] !== scrolled) {
                element.dataset["scrolled"] = scrolled;
              }
            }}
          >
            <div className="settings-split">
              <div className="settings-column">
                {searching ? (
                  <>
                    {found.count === 0 && (
                      <div className="settings-empty">No settings match “{query.trim()}”</div>
                    )}
                    {found.groups.map((group) => (
                      <Group key={`${group.page}:${group.label}`} label={group.label}>
                        {group.rows.map((row) => rowOf(row, () => go(group.page, row.path)))}
                      </Group>
                    ))}
                  </>
                ) : (
                  current && (
                    <>
                      {current.blurb !== "" && <p className="settings-intro">{current.blurb}</p>}
                      {current.groups
                        .map((group) => ({ ...group, rows: group.rows.filter((row) => row.shown) }))
                        .filter((group) => group.rows.length > 0)
                        .map((group) => (
                          <Group key={group.label} label={group.label}>
                            {group.rows.map((row) => rowOf(row))}
                          </Group>
                        ))}
                    </>
                  )
                )}
              </div>
              {/* The column beside the rows is a page's own to fill. */}
              {Preview && (
                <aside className="settings-preview" aria-label="Live preview">
                  <Preview />
                </aside>
              )}
            </div>
          </div>
        </main>
      </div>
      <Notices store={app.notices} />
    </div>
  );
}
