/**
 * The palette that opens a terminal from a profile, a local shell, a Linux distribution
 * or another machine over SSH, and a session of a Rex server.
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
 *
 * A Rex server is connected when it is chosen, and the palette stays to list its
 * sessions in the column. Choosing it then puts the marked session on show in place of
 * the workspace. Tab goes into the sessions, where the arrow keys move among them, and
 * back. A connection that would not be encrypted is said to be so in the column, and is
 * made only by the button there or by its keys.
 */

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useStore } from "zustand";

import { connectionDialog } from "../connections/store.ts";
import type { DetachedSession, HostProbe } from "../core/types.ts";
import { Glyph } from "../icons/Glyph.tsx";
import { homeShortened } from "../events/where.ts";
import { LinkDot } from "../rex/LinkDot.tsx";
import { useRexActions, useRexLook } from "../rex/look.ts";
import { connected, firstMarked, linkOf, next } from "../rex/model.ts";
import { useShell } from "../shell/context.ts";
import { FileIcon, RefreshIcon, SearchIcon } from "../shell/icons.tsx";
import { sentence } from "../shell/notices.ts";
import { Cap, KeyCaps } from "../palette/KeyCaps.tsx";
import { HostDetail, sessionOptionId, type HostActs } from "./HostDetail.tsx";
import { KindIcon } from "./icons.tsx";
import { readPins, togglePin } from "./pins.ts";
import {
  CONNECT_ACTIONS,
  connectFilters,
  connectGroups,
  connectSections,
  describeRun,
  detachedEntries,
  hostEntries,
  openCommand,
  reattachCommand,
  sourceOf,
  stepFilter,
  type ConnectAction,
  type ConnectEntry,
  type ConnectFilter,
  type ConnectRow,
  type DetachedEntry,
  type HostEntry,
} from "./model.ts";

const LIST_ID = "connect-list";

function optionId(index: number): string {
  return `connect-option-${index}`;
}

/** Tells one row from the others, whatever else the list holds. */
function keyOf(row: ConnectRow): string {
  switch (row.type) {
    case "host":
      return `host:${row.entry.host.id}`;
    case "profile":
      return `profile:${row.entry.profile.id}`;
    case "session":
      return `session:${row.entry.session.backend}:${row.entry.session.session}`;
    case "action":
      return `action:${row.action.id}`;
  }
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

/** The mark of a row that is pinned and listed where its kind is. */
function PinnedMark() {
  return (
    <span className="connect-row-pin" title="Pinned">
      <Glyph name="pin" size={11} />
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

/** What a host's measurement says, as the column and the row show it. */
function latencyOf(probe: HostProbe | "pending" | undefined): string {
  if (probe === undefined || probe === "pending") return "…";
  return probe.ms === null ? "—" : `${probe.ms} ms`;
}

/** The column that says what the marked row is. */
function Detail({
  row,
  splitKey,
  open,
  act,
  session,
  host,
  pin,
  probe,
  running,
}: {
  row: ConnectRow | undefined;
  splitKey: string;
  open: (entry: ConnectEntry, how: "tab" | "split") => void;
  act: (action: ConnectAction) => void;
  session: (entry: DetachedEntry) => ReactNode;
  host: (entry: HostEntry) => ReactNode;
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
  if (row.type === "host") return host(row.entry);
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
  // The marked row, by what it is: servers come and go while the palette is open, and
  // the mark stays on what it was on. Nothing means the first row.
  const [selected, setSelected] = useState<string | null>(null);
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
  const servers = useStore(app.rex, (state) => state.list);
  const backend = useStore(app.layout, (state) => state.backend);
  useRexLook();
  const rex = useRexActions();
  // Which session of the marked server is marked, once the user has marked one, and
  // whether the arrow keys move among them.
  const [session, setSession] = useState<{ host: string; id: string } | null>(null);
  const [inside, setInside] = useState(false);

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
  const hosts = useMemo(
    () => hostEntries(servers, query, filter, pinned),
    [servers, query, filter, pinned],
  );
  const sections = useMemo(
    () => connectSections(groups, sessions, hosts),
    [groups, sessions, hosts],
  );
  const rows = useMemo(() => sections.flatMap((section) => section.rows), [sections]);
  // How many of each kind match what is typed, for the chips.
  const counts = useMemo(() => {
    const every = connectGroups(list, query, "all");
    const of = (kind: ConnectFilter) =>
      every
        .filter((group) => kind === "all" || group.kind === kind)
        .reduce((sum, group) => sum + group.entries.length, 0);
    const loose = detachedEntries(detached, query, "detached", now, place).length;
    const served = hostEntries(servers, query, "rex").length;
    return (filter: ConnectFilter) => {
      switch (filter) {
        case "detached":
          return loose;
        case "rex":
          return served;
        case "all":
          return of("all") + loose + served;
        default:
          return of(filter);
      }
    };
  }, [list, query, detached, now, place, servers]);
  const marked = Math.max(
    0,
    rows.findIndex((row) => keyOf(row) === selected),
  );
  const mark = (index: number) => {
    const row = rows[index];
    if (row) setSelected(keyOf(row));
  };
  const first = rows[0];
  useEffect(() => {
    // Once the list is known its first row is the one marked, whatever comes before it.
    if (selected === null && first && list !== null && servers !== null) setSelected(keyOf(first));
  }, [selected, first, list, servers]);
  const splitKey = platform === "macos" ? "⌘↵" : "Ctrl+↵";

  useEffect(() => {
    results.current?.querySelector(`#${optionId(marked)}`)?.scrollIntoView({ block: "nearest" });
  }, [marked, rows]);

  // A host is measured when it is first marked, and once only.
  const markedRow = rows[marked];
  const markedHostRow = markedRow?.type === "host" ? markedRow.entry : null;
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
  const pinRow = (row: ConnectRow | undefined) => {
    if (row?.type === "profile") setPins(togglePin(row.entry.profile.id));
    if (row?.type === "host") setPins(togglePin(row.entry.host.id));
  };
  const pin = (entry: ConnectEntry) => pinRow({ type: "profile", entry });
  /** Which of a server's sessions is marked. */
  const markedSession = (entry: HostEntry): number => {
    const at =
      session?.host === entry.host.id
        ? entry.host.sessions.findIndex((found) => found.id === session.id)
        : -1;
    return at >= 0 ? at : firstMarked(entry.host, backend);
  };
  const markSession = (entry: HostEntry, index: number) => {
    const found = entry.host.sessions[index];
    if (found) setSession({ host: entry.host.id, id: found.id });
  };
  const canEnter =
    markedHostRow !== null &&
    connected(markedHostRow.host) &&
    markedHostRow.host.sessions.length > 0;
  const moving = inside && canEnter;

  // Putting the palette away gives the keyboard back to where the user was working.
  const close = () => app.connect.getState().close();
  const open = (entry: ConnectEntry, how: "tab" | "split") => {
    const command = openCommand(entry.profile.id, how, app.views.focused() !== undefined);
    close();
    app.commands.run(command.id, command.args);
  };
  const act = (action: ConnectAction) => {
    close();
    app.commands.run(action.command, action.args ?? null);
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
  // Putting a session on show takes the place of what the window shows, and the keyboard
  // with it: the palette is put away first.
  const onHost: HostActs = {
    attach: (entry, id) => {
      close();
      app.commands.run("lume.rex.open", { host: entry.host.id, session: id });
    },
    newSession: (entry) => {
      close();
      app.commands.run("lume.rex.new-session", { host: entry.host.id });
    },
    connect: (entry, agreed) => rex.connect(entry.host, agreed),
    disconnect: (entry) => rex.disconnect(entry.host),
    copy: (entry) => {
      app.client
        .clipboardWrite(entry.host.address)
        .catch((error: unknown) =>
          app.notices.push(
            "error",
            sentence(error instanceof Error ? error.message : String(error)),
          ),
        );
    },
    pin: (entry) => pinRow({ type: "host", entry }),
    edit: (entry) => {
      close();
      connectionDialog
        .getState()
        .edit({ kind: "rex", label: entry.host.label, address: entry.host.address }, false);
    },
  };
  /** What choosing a server does: attaches, makes a session, or connects. */
  const acceptHost = (entry: HostEntry, agreed: boolean) => {
    const { host } = entry;
    if (connected(host)) {
      const chosen = host.sessions[markedSession(entry)];
      if (chosen) onHost.attach(entry, chosen.id);
      else onHost.newSession(entry);
      return;
    }
    const step = next(host);
    // Agreeing has keys of its own: those that connect are pressed without reading.
    if (step && (step.kind === "connect" || agreed)) {
      onHost.connect(entry, step.kind === "agree");
    }
  };
  const accept = (index: number, how: "tab" | "split") => {
    const row = rows[index];
    if (!row) return;
    if (row.type === "profile") open(row.entry, how);
    else if (row.type === "session") attach(row.entry, how);
    else if (row.type === "host") acceptHost(row.entry, how === "split");
    else act(row.action);
  };
  const move = (to: number) => {
    if (moving && markedHostRow) {
      const count = markedHostRow.host.sessions.length;
      const by = to - marked;
      markSession(markedHostRow, (markedSession(markedHostRow) + by + count) % count);
      return;
    }
    if (rows.length === 0) return;
    mark(((to % rows.length) + rows.length) % rows.length);
  };
  const filterBy = (next: ConnectFilter) => {
    setFilter(next);
    setSelected(null);
    setInside(false);
  };
  const stepBy = (by: 1 | -1) => filterBy(stepFilter(filter, by, filters));

  const profileCount = rows.filter((row) => row.type === "profile").length;
  let index = 0;
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
          if (at === marked) return;
          mark(at);
          setInside(false);
        }}
        onClick={(event) => accept(at, event.ctrlKey || event.metaKey ? "split" : "tab")}
      >
        {children}
      </div>
    );
  };
  /** One row of the list, as its kind is drawn. */
  const draw = (row: ConnectRow, section: string) => {
    switch (row.type) {
      case "host": {
        const { entry } = row;
        return option(
          row,
          <>
            <span className="connect-row-icon" data-kind="rex">
              <Glyph name="server" size={15} />
            </span>
            <span className="connect-row-name">
              <MarkedName text={entry.host.label} indices={entry.indices} />
            </span>
            <LinkDot link={linkOf(entry.host)} className="connect-dot" />
            {entry.pinned && section !== "pinned" && <PinnedMark />}
            <span className="connect-row-line">{entry.line}</span>
            {entry.meta !== "" && <span className="connect-meta">{entry.meta}</span>}
          </>,
          "connect-row",
        );
      }
      case "profile": {
        const { entry } = row;
        return option(
          row,
          <>
            <span className="connect-row-icon" data-kind={entry.kind}>
              <KindIcon kind={entry.kind} />
            </span>
            <span className="connect-row-name">
              <MarkedName text={entry.profile.name} indices={entry.indices} />
            </span>
            {entry.pinned && section !== "pinned" && <PinnedMark />}
            <span className="connect-row-line" data-kind={entry.kind}>
              {entry.line}
            </span>
            {entry.isDefault && <span className="connect-meta">default</span>}
            {typeof probes.get(entry.profile.id) === "object" && (
              <span className="connect-meta">{latencyOf(probes.get(entry.profile.id))}</span>
            )}
          </>,
          "connect-row",
        );
      }
      case "session": {
        const { entry } = row;
        return option(
          row,
          <>
            <SessionDot session={entry.session} />
            <span className="connect-row-name">
              <MarkedName text={entry.name} indices={entry.indices} />
            </span>
            <span className="connect-row-line">{entry.line}</span>
            {markedRow === row && <span className="connect-meta">↵ attach</span>}
          </>,
          "connect-row",
        );
      }
      case "action":
        return option(
          row,
          <>
            <span className="connect-action-icon">
              {row.action.id === "add" ? (
                <Glyph name="plus" size={15} />
              ) : row.action.id === "config" ? (
                <FileIcon />
              ) : (
                <RefreshIcon />
              )}
            </span>
            <span className="connect-action-label">{row.action.label}</span>
            {row.action.keys !== undefined && (
              <KeyCaps
                label={platform === "macos" ? row.action.keys.mac : row.action.keys.pc}
                className="connect-keys"
              />
            )}
          </>,
          "connect-action",
        );
    }
  };
  // What Enter does now, for the line of keys.
  const enterDoes =
    markedHostRow === null
      ? "Open in new tab"
      : connected(markedHostRow.host)
        ? markedHostRow.host.sessions.length > 0
          ? "Attach"
          : "New session"
        : "Connect";

  const agrees = markedHostRow !== null && next(markedHostRow.host)?.kind === "agree";

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
            aria-activedescendant={
              moving && markedHostRow
                ? sessionOptionId(markedSession(markedHostRow))
                : rows.length > 0
                  ? optionId(marked)
                  : undefined
            }
            placeholder="Search shells, distributions and hosts"
            spellCheck={false}
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setSelected(null);
              setInside(false);
            }}
            onKeyDown={(event) => {
              const taken = () => {
                event.preventDefault();
                event.stopPropagation();
              };
              if (event.ctrlKey || event.metaKey) {
                const key = event.key.toLowerCase();
                if (key === "d" && !event.shiftKey) {
                  pinRow(markedRow);
                  taken();
                  return;
                }
                if (key === "a" && event.shiftKey) {
                  const add = CONNECT_ACTIONS.find((action) => action.id === "add");
                  if (add) act(add);
                  taken();
                  return;
                }
                if (markedHostRow && key === "n" && !event.shiftKey) {
                  if (connected(markedHostRow.host)) onHost.newSession(markedHostRow);
                  taken();
                  return;
                }
                if (markedHostRow && key === "c" && event.shiftKey) {
                  onHost.copy(markedHostRow);
                  taken();
                  return;
                }
              }
              if (event.altKey && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
                stepBy(event.key === "ArrowLeft" ? -1 : 1);
                taken();
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
                  setInside(false);
                  mark(0);
                  break;
                case "End":
                  setInside(false);
                  mark(rows.length - 1);
                  break;
                case "Tab":
                  // Into the sessions of a server that has some, and back.
                  if (canEnter) setInside(!inside);
                  else stepBy(event.shiftKey ? -1 : 1);
                  break;
                case "Enter":
                  accept(marked, event.ctrlKey || event.metaKey ? "split" : "tab");
                  break;
                case "Backspace":
                  // An empty field goes back to listing every kind.
                  if (query !== "" || filter === "all") {
                    event.stopPropagation();
                    return;
                  }
                  filterBy("all");
                  break;
                case "Escape":
                  if (moving) setInside(false);
                  else close();
                  break;
                default:
                  // What is typed here is the query, not a key for a terminal.
                  event.stopPropagation();
                  return;
              }
              taken();
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
              ) : choice.value === "rex" ? (
                <Glyph name="server" size={12} />
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
              {sections.map((section) => (
                <div
                  key={section.key}
                  className="connect-group"
                  role="group"
                  aria-label={section.label}
                >
                  <div className="connect-heading" aria-hidden="true">
                    {section.label}
                  </div>
                  {section.rows.map((row) => draw(row, section.key))}
                </div>
              ))}
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
              : filter === "rex"
                ? hosts.length === 0 && (
                    <p className="connect-note">
                      {query.trim() !== "" && servers !== null && servers.hosts.length > 0
                        ? "No server matches."
                        : "No Rex server is known. Add one below."}
                    </p>
                  )
                : profileCount === 0 &&
                  sessions.length === 0 &&
                  hosts.length === 0 && (
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
            host={(entry) => (
              <HostDetail
                key={entry.host.id}
                entry={entry}
                backend={backend}
                marked={markedSession(entry)}
                inside={moving}
                problem={rex.problem(entry.host)}
                agreeKey={splitKey}
                mac={platform === "macos"}
                acts={onHost}
                mark={(index) => markSession(entry, index)}
              />
            )}
          />
        </div>
        <div className="connect-footer">
          <span>
            {agrees ? <KeyCaps label={splitKey} className="connect-footer-keys" /> : <Cap>↵</Cap>}
            {enterDoes}
          </span>
          {markedHostRow === null ? (
            <span>
              <KeyCaps label={splitKey} className="connect-footer-keys" />
              Split right
            </span>
          ) : (
            <span data-off={!canEnter || undefined}>
              <Cap>Tab</Cap>
              {moving ? "Back to hosts" : "Sessions"}
            </span>
          )}
          <span>
            <Cap>↑</Cap>
            <Cap>↓</Cap>
            Move
          </span>
          <span>
            <KeyCaps
              label={platform === "macos" ? "⌥←→" : "Alt+←→"}
              className="connect-footer-keys"
            />
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
