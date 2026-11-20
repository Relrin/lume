/**
 * The palette that opens a terminal from a profile: a local shell, a Linux distribution,
 * another machine over SSH.
 *
 * It opens over the window with the keyboard in its field. Typing narrows the list, the
 * filters along the field show one kind of profile, the arrow keys move through the
 * list, and the column beside it says what the marked profile will run and where it
 * comes from. Enter opens the profile in a new tab and Ctrl+Enter beside the pane that
 * has the keyboard; Escape puts the palette away. The filters stand in a row of chips
 * under the field, each with how many of its kind match what is typed.
 *
 * Ctrl+D, or the pin beside a profile's name in the column, pins the marked profile:
 * pinned profiles are listed first. For a host reached over SSH the column also says
 * how long a connection to it takes to open, measured when the host is first marked,
 * how many panes of the window run it, and its port.
 *
 * Sessions that still run in the session host with no pane to show them are listed
 * after the profiles, and are shown again in a tab of their own, or beside the pane
 * that has the keyboard, the same way.
 */

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useStore } from "zustand";

import type { DetachedSession, HostProbe } from "../core/types.ts";
import { Glyph } from "../icons/Glyph.tsx";
import { homeShortened } from "../events/where.ts";
import { useShell } from "../shell/context.ts";
import { FileIcon, RefreshIcon, SearchIcon } from "../shell/icons.tsx";
import { sentence } from "../shell/notices.ts";
import { Cap, KeyCaps } from "../palette/KeyCaps.tsx";
import { KindIcon } from "./icons.tsx";
import { readPins, togglePin } from "./pins.ts";
import {
  DETACHED_LABEL,
  connectFilters,
  connectGroups,
  connectRows,
  describeRun,
  detachedEntries,
  openCommand,
  reattachCommand,
  sourceOf,
  stepFilter,
  type ConnectAction,
  type ConnectEntry,
  type ConnectFilter,
  type ConnectRow,
  type DetachedEntry,
} from "./model.ts";

const LIST_ID = "connect-list";

function optionId(index: number): string {
  return `connect-option-${index}`;
}

/** A profile's name with the characters the query matched marked. */
function MarkedName({ text, indices }: { text: string; indices: readonly number[] }) {
  if (indices.length === 0) return <>{text}</>;
  const marked = new Set(indices);
  const runs: { text: string; marked: boolean }[] = [];
  [...text].forEach((character, index) => {
    const isMarked = marked.has(index);
    const last = runs[runs.length - 1];
    if (last && last.marked === isMarked) last.text += character;
    else runs.push({ text: character, marked: isMarked });
  });
  return (
    <>
      {runs.map((run, index) =>
        run.marked ? (
          <mark key={index} className="connect-mark">
            {run.text}
          </mark>
        ) : (
          <span key={index}>{run.text}</span>
        ),
      )}
    </>
  );
}

/** Whether a detached session's program runs, as a dot like the mockup's. */
function SessionDot({ session }: { session: DetachedSession }) {
  return (
    <span className="connect-row-icon">
      <span className="connect-dot" data-state={session.exited ? "ended" : "running"} />
    </span>
  );
}

/** What the column beside the list says of a detached session. */
function SessionDetail({
  entry,
  place,
  attach,
  end,
}: {
  entry: DetachedEntry;
  place: (cwd: string) => string;
  attach: (entry: DetachedEntry, how: "tab" | "split") => void;
  end: (entry: DetachedEntry) => void;
}) {
  const { session } = entry;
  const facts: [string, string][] = [["Runs", session.program]];
  if (session.cwd !== null && session.cwd !== "") facts.push(["In", place(session.cwd)]);
  if (session.pid !== null) facts.push(["Process", String(session.pid)]);
  facts.push(["Started", new Date(session.created_unix_ms).toLocaleString()]);
  facts.push(["Session", session.session]);
  return (
    <section className="connect-detail" aria-label="Selection">
      <div className="connect-detail-head">
        <h2 className="connect-detail-name">{entry.name}</h2>
        <span className="connect-detail-line">
          {session.exited ? "Ended, not shown in any pane" : "Running, not shown in any pane"}
        </span>
      </div>
      <dl className="connect-facts">
        {facts.map(([label, value]) => (
          <div key={label} className="connect-fact">
            <dt>{label}</dt>
            <dd data-prose={label === "From" || undefined}>{value}</dd>
          </div>
        ))}
      </dl>
      <div className="connect-detail-spacer" />
      <div className="connect-acts">
        <button
          type="button"
          className="connect-primary"
          tabIndex={-1}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => attach(entry, "tab")}
        >
          Attach in new tab
          <KeyCaps label="↵" className="connect-keys" />
        </button>
        <button
          type="button"
          className="connect-secondary"
          tabIndex={-1}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => end(entry)}
        >
          End session
        </button>
      </div>
    </section>
  );
}

/** The column that says what the marked profile, session or action is. */
/** What a host's measurement says, as the column and the row show it. */
function latencyOf(probe: HostProbe | "pending" | undefined): string {
  if (probe === undefined || probe === "pending") return "…";
  return probe.ms === null ? "—" : `${probe.ms} ms`;
}

function Detail({
  row,
  splitKey,
  open,
  act,
  session,
  pin,
  probe,
  running,
}: {
  row: ConnectRow | undefined;
  splitKey: string;
  open: (entry: ConnectEntry, how: "tab" | "split") => void;
  act: (action: ConnectAction) => void;
  session: (entry: DetachedEntry) => ReactNode;
  /** Pins the profile, or takes its pin away. */
  pin: (entry: ConnectEntry) => void;
  /** What measuring a host found, by profile id. */
  probe: (profile: string) => HostProbe | "pending" | undefined;
  /** How many panes of the window run a profile. */
  running: (profile: string) => number;
}) {
  if (!row) {
    return (
      <section className="connect-detail" aria-label="Selection">
        <p className="connect-detail-empty">Nothing is marked.</p>
      </section>
    );
  }
  if (row.type === "session") return session(row.entry);
  if (row.type === "action") {
    return (
      <section className="connect-detail" aria-label="Selection">
        <h2 className="connect-detail-name">{row.action.label}</h2>
        <p className="connect-detail-about">{row.action.about}</p>
        <div className="connect-detail-spacer" />
        <div className="connect-acts">
          <button
            type="button"
            className="connect-primary"
            tabIndex={-1}
            onPointerDown={(event) => event.preventDefault()}
            onClick={() => act(row.action)}
          >
            {row.action.label}
            <KeyCaps label="↵" className="connect-keys" />
          </button>
        </div>
      </section>
    );
  }
  const { entry } = row;
  const { profile } = entry;
  const facts: [string, string][] = [
    ["Runs", describeRun(profile)],
    ["From", sourceOf(profile)],
  ];
  if (profile.cwd !== undefined) facts.push(["Starts in", profile.cwd]);
  facts.push(["Id", profile.id]);
  const launcher = profile.launcher;
  const measured = launcher.type === "ssh" ? probe(profile.id) : undefined;
  const stats: [string, string, string | undefined][] =
    launcher.type === "ssh"
      ? [
          [
            "Latency",
            latencyOf(measured),
            typeof measured === "object" ? measured.note : undefined,
          ],
          ["Panes", String(running(profile.id)), undefined],
          ["Port", String(launcher.port ?? 22), undefined],
        ]
      : [];
  return (
    <section className="connect-detail" aria-label="Selection">
      <div className="connect-detail-top">
        <div className="connect-detail-head">
          <h2 className="connect-detail-name">{profile.name}</h2>
          <span className="connect-detail-line">{entry.line}</span>
        </div>
        <button
          type="button"
          className="connect-pin"
          aria-pressed={entry.pinned}
          aria-label={entry.pinned ? `Unpin ${profile.name}` : `Pin ${profile.name}`}
          title={entry.pinned ? "Unpin (Ctrl+D)" : "Pin to the top (Ctrl+D)"}
          tabIndex={-1}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => pin(entry)}
        >
          <Glyph name="pin" size={14} />
        </button>
      </div>
      {stats.length > 0 && (
        <dl className="connect-stats">
          {stats.map(([label, value, note]) => (
            <div key={label} title={note}>
              <dt>{label}</dt>
              <dd data-prose={label === "From" || undefined}>{value}</dd>
            </div>
          ))}
        </dl>
      )}
      <dl className="connect-facts">
        {facts.map(([label, value]) => (
          <div key={label} className="connect-fact">
            <dt>{label}</dt>
            <dd data-prose={label === "From" || undefined}>{value}</dd>
          </div>
        ))}
      </dl>
      <div className="connect-detail-spacer" />
      <div className="connect-acts">
        <button
          type="button"
          className="connect-primary"
          tabIndex={-1}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => open(entry, "tab")}
        >
          Open in new tab
          <KeyCaps label="↵" className="connect-keys" />
        </button>
        <button
          type="button"
          className="connect-secondary"
          tabIndex={-1}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => open(entry, "split")}
        >
          Split right
          <KeyCaps label={splitKey} className="connect-keys" />
        </button>
      </div>
    </section>
  );
}

function ConnectDialog() {
  const app = useShell();
  const list = useStore(app.profiles, (state) => state.list);
  const platform = useStore(app.config, (state) => state.view?.platform);
  const opened = useStore(app.connect, (state) => state.filter);
  const field = useRef<HTMLInputElement>(null);
  const results = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<ConnectFilter>(opened);
  const [selected, setSelected] = useState(0);
  // The sessions no pane shows: `null` until the core has said, and if it could not.
  const [detached, setDetached] = useState<readonly DetachedSession[] | null>(null);
  const [detachedProblem, setDetachedProblem] = useState<string | null>(null);
  const [asked, setAsked] = useState(0);
  const [pins, setPins] = useState<readonly string[]>(() => readPins());
  const pinned = useMemo(() => new Set(pins), [pins]);
  // What measuring each host found, kept while the palette is open.
  const [probes, setProbes] = useState<ReadonlyMap<string, HostProbe | "pending">>(() => new Map());
  // When the list was read, which is what the sessions' ages are counted from.
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    field.current?.focus();
  }, []);

  useEffect(() => {
    let current = true;
    app.client.sessionsDetached().then(
      (answer) => {
        if (!current) return;
        setDetached(answer.sessions);
        setDetachedProblem(null);
        setNow(Date.now());
      },
      (error: unknown) => {
        if (!current) return;
        setDetached(null);
        setDetachedProblem(error instanceof Error ? error.message : String(error));
      },
    );
    return () => {
      current = false;
    };
  }, [app.client, asked]);

  const windows = platform === "windows";
  const place = useMemo(
    () => (cwd: string) => homeShortened(cwd, app.machine.home, windows),
    [app.machine.home, windows],
  );
  const filters = connectFilters((detached?.length ?? 0) > 0, filter);
  const groups = useMemo(
    () => connectGroups(list, query, filter, pinned),
    [list, query, filter, pinned],
  );
  const sessions = useMemo(
    () => detachedEntries(detached, query, filter, now, place),
    [detached, query, filter, now, place],
  );
  const rows = useMemo(() => connectRows(groups, sessions), [groups, sessions]);
  // How many of each kind match what is typed, for the chips.
  const counts = useMemo(() => {
    const every = connectGroups(list, query, "all");
    const of = (kind: ConnectFilter) =>
      every
        .filter((group) => kind === "all" || group.kind === kind)
        .reduce((sum, group) => sum + group.entries.length, 0);
    const loose = detachedEntries(detached, query, "detached", now, place).length;
    return (filter: ConnectFilter) =>
      filter === "detached" ? loose : filter === "all" ? of("all") + loose : of(filter);
  }, [list, query, detached, now, place]);
  const marked = Math.min(selected, Math.max(0, rows.length - 1));
  const splitKey = platform === "macos" ? "⌘↵" : "Ctrl+↵";

  useEffect(() => {
    results.current?.querySelector(`#${optionId(marked)}`)?.scrollIntoView({ block: "nearest" });
  }, [marked, rows]);

  // A host is measured when it is first marked, and once only.
  const markedRow = rows[marked];
  const markedHost =
    markedRow?.type === "profile" && markedRow.entry.profile.launcher.type === "ssh"
      ? markedRow.entry.profile.id
      : null;
  useEffect(() => {
    if (markedHost === null || probes.has(markedHost)) return;
    setProbes((held) => new Map(held).set(markedHost, "pending"));
    app.client.hostProbe(markedHost).then(
      (found) => setProbes((held) => new Map(held).set(markedHost, found)),
      (error: unknown) =>
        setProbes((held) =>
          new Map(held).set(markedHost, {
            ms: null,
            note: error instanceof Error ? error.message : String(error),
          }),
        ),
    );
  }, [app.client, markedHost, probes]);

  /** How many panes of the window run a profile. */
  const running = (profile: string): number => {
    const state = app.layout.getState();
    if (!state.ready) return 0;
    return Object.values(state.panes).filter(
      (pane) => pane?.content.type === "terminal" && pane.content.profile === profile,
    ).length;
  };
  const pin = (entry: ConnectEntry) => setPins(togglePin(entry.profile.id));

  // Putting the palette away gives the keyboard back to where the user was working.
  const close = () => app.connect.getState().close();
  const open = (entry: ConnectEntry, how: "tab" | "split") => {
    const command = openCommand(entry.profile.id, how, app.views.focused() !== undefined);
    close();
    app.commands.run(command.id, command.args);
  };
  const act = (action: ConnectAction) => {
    close();
    app.commands.run(action.command);
  };
  const attach = (entry: DetachedEntry, how: "tab" | "split") => {
    const command = reattachCommand(entry.session, how);
    close();
    app.commands.run(command.id, command.args);
  };
  // The palette stays, and lists the sessions again once the session has gone.
  const end = (entry: DetachedEntry) => {
    const { session, backend } = entry.session;
    app.client.commandRun("lume.session.end", { session, backend }).then(
      () => setAsked((count) => count + 1),
      (error: unknown) => {
        app.notices.push("error", sentence(error instanceof Error ? error.message : String(error)));
        setAsked((count) => count + 1);
      },
    );
  };
  const accept = (index: number, how: "tab" | "split") => {
    const row = rows[index];
    if (!row) return;
    if (row.type === "profile") open(row.entry, how);
    else if (row.type === "session") attach(row.entry, how);
    else act(row.action);
  };
  const move = (to: number) => {
    if (rows.length === 0) return;
    setSelected(((to % rows.length) + rows.length) % rows.length);
  };
  const filterBy = (next: ConnectFilter) => {
    setFilter(next);
    setSelected(0);
  };

  const profileCount = rows.filter((row) => row.type === "profile").length;
  let index = 0;
  const keyOf = (row: ConnectRow): string => {
    switch (row.type) {
      case "profile":
        return `profile:${row.entry.profile.id}`;
      case "session":
        return `session:${row.entry.session.backend}:${row.entry.session.session}`;
      case "action":
        return `action:${row.action.id}`;
    }
  };
  const option = (row: ConnectRow, children: ReactNode, className: string) => {
    const at = index++;
    return (
      <div
        key={keyOf(row)}
        id={optionId(at)}
        className={className}
        role="option"
        aria-selected={at === marked}
        onPointerMove={() => {
          if (at !== marked) setSelected(at);
        }}
        onClick={(event) => accept(at, event.ctrlKey || event.metaKey ? "split" : "tab")}
      >
        {children}
      </div>
    );
  };

  return (
    <div className="connect-backdrop" onPointerDown={close}>
      <div
        className="connect"
        role="dialog"
        aria-modal="true"
        aria-label="Open a terminal"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <div className="connect-field">
          <span className="connect-field-icon">
            <SearchIcon />
          </span>
          <input
            ref={field}
            className="connect-input"
            type="text"
            role="combobox"
            aria-label="Search shells, distributions and hosts"
            aria-expanded={rows.length > 0}
            aria-controls={LIST_ID}
            aria-autocomplete="list"
            aria-activedescendant={rows.length > 0 ? optionId(marked) : undefined}
            placeholder="Search shells, distributions and hosts"
            spellCheck={false}
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setSelected(0);
            }}
            onKeyDown={(event) => {
              if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "d") {
                const row = rows[marked];
                if (row?.type === "profile") pin(row.entry);
                event.preventDefault();
                event.stopPropagation();
                return;
              }
              switch (event.key) {
                case "ArrowDown":
                  move(marked + 1);
                  break;
                case "ArrowUp":
                  move(marked - 1);
                  break;
                case "Home":
                  move(0);
                  break;
                case "End":
                  move(rows.length - 1);
                  break;
                case "Tab":
                  filterBy(stepFilter(filter, event.shiftKey ? -1 : 1, filters));
                  break;
                case "Enter":
                  accept(marked, event.ctrlKey || event.metaKey ? "split" : "tab");
                  break;
                case "Escape":
                  close();
                  break;
                default:
                  // What is typed here is the query, not a key for a terminal.
                  event.stopPropagation();
                  return;
              }
              event.preventDefault();
              event.stopPropagation();
            }}
          />
        </div>
        <div className="connect-chips" role="group" aria-label="Show">
          {filters.map((choice) => (
            <button
              key={choice.value}
              type="button"
              className="connect-chip"
              data-filter={choice.value}
              aria-label={`${choice.label}, ${counts(choice.value)}`}
              aria-pressed={filter === choice.value}
              tabIndex={-1}
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => filterBy(choice.value)}
            >
              {choice.value === "local" || choice.value === "wsl" || choice.value === "ssh" ? (
                <KindIcon kind={choice.value} size={12} />
              ) : null}
              {choice.label}
              <span className="connect-chip-count" aria-hidden="true">
                {counts(choice.value)}
              </span>
            </button>
          ))}
        </div>
        <div className="connect-body">
          <div ref={results} className="connect-results">
            <div id={LIST_ID} className="connect-list" role="listbox" aria-label="Profiles">
              {groups.map((group) => (
                <div
                  key={group.kind}
                  className="connect-group"
                  role="group"
                  aria-label={group.label}
                >
                  <div className="connect-heading" aria-hidden="true">
                    {group.label}
                  </div>
                  {group.entries.map((entry) =>
                    option(
                      { type: "profile", entry },
                      <>
                        <span className="connect-row-icon" data-kind={entry.kind}>
                          <KindIcon kind={entry.kind} />
                        </span>
                        <span className="connect-row-name">
                          <MarkedName text={entry.profile.name} indices={entry.indices} />
                        </span>
                        {entry.pinned && group.kind !== "pinned" && (
                          <span className="connect-row-pin" title="Pinned">
                            <Glyph name="pin" size={11} />
                          </span>
                        )}
                        <span className="connect-row-line" data-kind={entry.kind}>
                          {entry.line}
                        </span>
                        {entry.isDefault && <span className="connect-meta">default</span>}
                        {typeof probes.get(entry.profile.id) === "object" && (
                          <span className="connect-meta">
                            {latencyOf(probes.get(entry.profile.id))}
                          </span>
                        )}
                      </>,
                      "connect-row",
                    ),
                  )}
                </div>
              ))}
              {sessions.length > 0 && (
                <div className="connect-group" role="group" aria-label={DETACHED_LABEL}>
                  <div className="connect-heading" aria-hidden="true">
                    {DETACHED_LABEL}
                  </div>
                  {sessions.map((entry) => {
                    const isMarked =
                      rows[marked]?.type === "session" && rows[marked].entry === entry;
                    return option(
                      { type: "session", entry },
                      <>
                        <SessionDot session={entry.session} />
                        <span className="connect-row-name">
                          <MarkedName text={entry.name} indices={entry.indices} />
                        </span>
                        <span className="connect-row-line">{entry.line}</span>
                        {isMarked && <span className="connect-meta">↵ attach</span>}
                      </>,
                      "connect-row",
                    );
                  })}
                </div>
              )}
              <div className="connect-group" role="group" aria-label="Actions">
                <div className="connect-heading" aria-hidden="true">
                  Actions
                </div>
                {rows
                  .filter((row) => row.type === "action")
                  .map((row) =>
                    option(
                      row,
                      <>
                        <span className="connect-action-icon">
                          {row.action.id === "config" ? <FileIcon /> : <RefreshIcon />}
                        </span>
                        <span className="connect-action-label">{row.action.label}</span>
                      </>,
                      "connect-action",
                    ),
                  )}
              </div>
            </div>
            {filter === "detached"
              ? sessions.length === 0 && (
                  <p className="connect-note">
                    {detachedProblem !== null
                      ? `The detached sessions could not be listed: ${detachedProblem}`
                      : detached === null
                        ? "Looking for detached sessions…"
                        : "No session runs without a pane to show it."}
                  </p>
                )
              : profileCount === 0 &&
                sessions.length === 0 && (
                  <p className="connect-note">
                    {list === null
                      ? "Looking for shells, distributions and hosts…"
                      : "No profile matches."}
                  </p>
                )}
            {list !== null && !list.looked && (
              <p className="connect-note">Still looking for shells, distributions and hosts…</p>
            )}
            {list?.notes.map((note) => (
              <p key={note} className="connect-note">
                {note}
              </p>
            ))}
          </div>
          <Detail
            row={rows[marked]}
            splitKey={splitKey}
            pin={pin}
            probe={(profile) => probes.get(profile)}
            running={running}
            open={open}
            act={act}
            session={(entry) => (
              <SessionDetail entry={entry} place={place} attach={attach} end={end} />
            )}
          />
        </div>
        <div className="connect-footer">
          <span>
            <Cap>↵</Cap>
            Open in new tab
          </span>
          <span>
            <KeyCaps label={splitKey} className="connect-footer-keys" />
            Split right
          </span>
          <span>
            <Cap>↑</Cap>
            <Cap>↓</Cap>
            Move
          </span>
          <span>
            <Cap>Tab</Cap>
            Kind
          </span>
          <span>
            <KeyCaps
              label={platform === "macos" ? "⌘D" : "Ctrl+D"}
              className="connect-footer-keys"
            />
            Pin
          </span>
          <span className="connect-footer-spacer" />
          <span>
            <Cap>Esc</Cap>
            Close
          </span>
        </div>
      </div>
    </div>
  );
}

/** The Connect palette, while it is on show. */
export function ConnectPalette() {
  const app = useShell();
  const shown = useStore(app.connect, (state) => state.shown);
  const opening = useStore(app.connect, (state) => state.opening);
  if (!shown) return null;
  // Each opening is a new dialog: nothing of the last one's query or filter is left.
  return <ConnectDialog key={opening} />;
}
