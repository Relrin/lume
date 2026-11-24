/**
 * The selector under the workspace's name: the hosts at the left, this machine and the
 * Rex servers, and what the one looked at holds at the right, workspaces of the
 * application's own or a server's sessions.
 *
 * The keyboard stays in its field. Typing narrows both columns, and what is typed and
 * not there can be made; the arrow keys move in one column, Tab goes to the other, Enter
 * opens, F2 renames in place and Escape puts the selector away. A host under the pointer
 * is looked at, which connects to nothing: a server is connected by its button. A
 * connection that would not be encrypted is said to be so, and is made by its button or
 * by keys of its own.
 *
 * Below the hosts stands the way to add one. A server the user added is called something
 * else in place too, and is changed from there while it is not connected.
 *
 * Ending a session or deleting a workspace asks first. Whatever needs something else
 * asked for is handed to the command palette, which asks.
 */

import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { useStore } from "zustand";

import { labelTaken } from "../connections/model.ts";
import { connectionDialog } from "../connections/store.ts";
import type { WorkspaceList } from "../core/types.ts";
import { Glyph } from "../icons/Glyph.tsx";
import { KeyCaps } from "../palette/KeyCaps.tsx";
import { LinkDot } from "../rex/LinkDot.tsx";
import { useRexActions, useRexLook } from "../rex/look.ts";
import { reason, usable } from "../rex/model.ts";
import { useShell } from "./context.ts";
import {
  browsedHost,
  firstRow,
  selectorActs,
  selectorHosts,
  selectorRows,
  THIS_PC,
  type SelectorAct,
  type SelectorHost,
  type SelectorRow,
  type SelectorSources,
} from "./host-selector.ts";
import { SearchIcon } from "./icons.tsx";

const HOSTS_ID = "selector-hosts";
const ROWS_ID = "selector-rows";
const ADD_ID = "selector-add";
/** What is marked among the hosts when it is the way to add one. */
const ADD = "add";

/** A row being renamed in place, and what is typed for it. */
interface Renaming {
  readonly key: string;
  readonly value: string;
}

/** Tells one action of the right column from the others. */
function actKey(act: SelectorAct): string {
  return act.kind === "command" ? `act:${act.command}` : `act:${act.kind}`;
}

/** What the arrow keys stop at in the right column: what is listed, then what can be done. */
type Stop =
  | { readonly kind: "row"; readonly key: string; readonly row: SelectorRow }
  | { readonly kind: "act"; readonly key: string; readonly act: SelectorAct };

/** The field a row is renamed in. */
function RenameField({
  label,
  value,
  taken,
  set,
  save,
  cancel,
}: {
  label: string;
  value: string;
  /** Whether another row is called so. */
  taken: boolean;
  set: (value: string) => void;
  save: () => void;
  cancel: () => void;
}) {
  const field = useRef<HTMLInputElement>(null);
  useEffect(() => {
    field.current?.focus();
    field.current?.select();
  }, []);
  const wrong = value.trim() === "" ? "A name is needed" : taken ? "That name is taken" : null;
  return (
    <input
      ref={field}
      className="selector-rename"
      aria-label={label}
      aria-invalid={wrong !== null}
      title={wrong ?? "Enter to save, Escape to cancel"}
      spellCheck={false}
      autoComplete="off"
      value={value}
      onChange={(event) => set(event.target.value)}
      onBlur={cancel}
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.key === "Enter") {
          event.preventDefault();
          if (wrong === null) save();
        } else if (event.key === "Escape") {
          event.preventDefault();
          cancel();
        }
      }}
    />
  );
}

/**
 * The selector, open. `close` puts it away and gives the keyboard back; `dismiss` puts it
 * away when something else has the keyboard already.
 */
export function HostSelector({
  keyboard,
  browse,
  close,
  dismiss,
}: {
  keyboard: boolean;
  /** The label of the server to look at first, in place of what is on show. */
  browse: string | null;
  close: () => void;
  dismiss: () => void;
}) {
  const app = useShell();
  const servers = useStore(app.rex, (state) => state.list);
  const backend = useStore(app.layout, (state) => state.backend);
  const known = useStore(app.commandStore, (state) => state.byId);
  const mac = useStore(app.config, (state) => state.view?.platform === "macos");
  const overlaid = useStore(app.palette, (state) => state.shown);
  const connecting = useStore(app.connect, (state) => state.shown);
  useRexLook();
  const rex = useRexActions();
  const box = useRef<HTMLDivElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const [workspaces, setWorkspaces] = useState<WorkspaceList | null>(null);
  const [query, setQuery] = useState("");
  const [wanted, setWanted] = useState<string | null>(
    () => servers?.hosts.find((found) => found.label === browse)?.id ?? backend,
  );
  const [column, setColumn] = useState<"hosts" | "rows">("rows");
  // Whether the mark among the hosts is on the way to add one.
  const [adding, setAdding] = useState(false);
  // What is marked at the right, by its key. Nothing means what is on show.
  const [stopAt, setStopAt] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<Renaming | null>(null);

  // Read as the selector opens: workspaces are made and removed from other windows and
  // from outside the application.
  useEffect(() => {
    let current = true;
    app.client.workspacesList().then(
      (list) => {
        if (current) setWorkspaces(list);
      },
      () => {
        // Without the list there is still what can be done with workspaces.
        if (current) setWorkspaces({ active: "", workspaces: [] });
      },
    );
    return () => {
      current = false;
    };
  }, [app]);

  useEffect(() => {
    field.current?.focus();
  }, []);

  // A palette that opens takes the place of the selector, and the keyboard.
  useEffect(() => {
    if (overlaid || connecting) dismiss();
  }, [overlaid, connecting, dismiss]);

  const sources: SelectorSources = { workspaces, servers, backend };
  const hosts = selectorHosts(sources, query);
  const host = browsedHost(sources, query, wanted);
  const rows = host ? selectorRows(sources, host, query) : [];
  const acts = host ? selectorActs(sources, host, query, (command) => known.has(command)) : [];
  const stops: readonly Stop[] = [
    ...rows.map((row): Stop => ({ kind: "row", key: row.key, row })),
    ...acts.map((act): Stop => ({ kind: "act", key: actKey(act), act })),
  ];
  const found = stops.findIndex((stop) => stop.key === stopAt);
  const first = host && query.trim() === "" && rows.length > 0 ? firstRow(sources, host) : 0;
  const marked = found >= 0 ? found : Math.min(first, Math.max(0, stops.length - 1));
  const at = stops[marked];
  const hostAt = hosts.findIndex((listed) => listed.key === host?.key);
  const agreeKey = mac ? "⌘↵" : "Ctrl+↵";
  const optionId = (index: number) => `selector-option-${index}`;
  const hostId = (index: number) => `selector-host-${index}`;

  const markedId = column === "hosts" ? (adding ? ADD_ID : hostId(hostAt)) : optionId(marked);
  useEffect(() => {
    box.current?.querySelector(`#${CSS.escape(markedId)}`)?.scrollIntoView({ block: "nearest" });
  }, [markedId]);

  // A server that was added just now may be listed only after the selector opened.
  const sought = servers?.hosts.find((listed) => listed.label === browse)?.id;
  useEffect(() => {
    if (sought !== undefined) setWanted(sought);
  }, [sought]);

  const look = (key: string) => {
    setAdding(false);
    if (key === host?.key) return;
    setWanted(key);
    setStopAt(null);
    setRenaming(null);
  };
  const leaveWith = (run: () => void) => {
    close();
    run();
  };
  const add = (byKey: boolean) => {
    dismiss();
    connectionDialog.getState().add("rex", byKey);
  };
  /** Changes how the server looked at is reached. */
  const edit = (byKey: boolean) => {
    const server = host?.rex;
    if (!server) return;
    dismiss();
    connectionDialog
      .getState()
      .edit({ kind: "rex", label: server.label, address: server.address }, byKey);
  };
  const openRow = (row: SelectorRow) => {
    if (!host) return;
    leaveWith(() => {
      if (host.rex) {
        app.commands.run("lume.rex.open", { host: host.rex.id, session: row.id });
      } else if (!row.on) {
        app.commands.run("lume.workspace.switch", { name: row.id });
      }
    });
  };
  const act = (what: SelectorAct, agreed: boolean, byKey = false) => {
    if (!host) return;
    const server = host.rex;
    switch (what.kind) {
      case "create":
        leaveWith(() => {
          if (server) {
            app.commands.run("lume.rex.new-session", { host: server.id, label: what.name });
          } else app.commands.run("lume.workspace.new", { name: what.name });
        });
        return;
      case "new-session":
        if (server) {
          leaveWith(() => app.commands.run("lume.rex.new-session", { host: server.id }));
        }
        return;
      case "connect":
        // Agreeing has keys of its own: those that connect are pressed without reading.
        if (server && (agreed || !what.agree)) rex.connect(server, what.agree);
        return;
      case "disconnect":
        if (server) rex.disconnect(server);
        return;
      case "edit":
        edit(byKey);
        return;
      case "command":
        leaveWith(() => {
          if (what.command === "lume.workspace.open-folder") app.commands.run(what.command);
          else app.palette.getState().openFor(what.command);
        });
        return;
    }
  };
  const take = (row: SelectorRow) => {
    if (!host || row.takes === null) return;
    const server = host.rex;
    leaveWith(() => {
      if (server) {
        // The core asks before it ends a session.
        app.commands.run("lume.rex.end-session", { host: server.id, session: row.id });
        return;
      }
      const name = row.id;
      void app.prompts
        .confirm({
          title: `Delete “${name}”?`,
          message: "The workspace's arrangement of tabs and panes is deleted. Its folder is not.",
          confirm: "Delete workspace",
        })
        .then((agreed) => {
          if (agreed) app.commands.run("lume.workspace.delete", { name });
          else app.views.focus();
        });
    });
  };
  const startRename = (row: SelectorRow) => {
    if (row.renames) setRenaming({ key: row.key, value: row.name });
  };
  const stopRename = () => {
    setRenaming(null);
    field.current?.focus();
  };
  const saveRename = (row: SelectorRow, value: string) => {
    const name = value.trim();
    stopRename();
    if (name === row.name) return;
    if (host?.rex) {
      rex.rename(host.rex, row.id, name);
      return;
    }
    app.client.commandRun("lume.workspace.rename", { name }).then(
      () =>
        app.client.workspacesList().then(
          (list) => setWorkspaces(list),
          () => undefined,
        ),
      (error: unknown) =>
        app.notices.push("error", error instanceof Error ? error.message : String(error)),
    );
  };

  const startHostRename = (listed: SelectorHost) => {
    if (renames(listed)) setRenaming({ key: listed.key, value: listed.name });
  };
  const saveHostRename = (listed: SelectorHost, value: string) => {
    const label = value.trim();
    stopRename();
    const server = listed.rex;
    if (!server || label === listed.name) return;
    app.client
      .rexHostPut(listed.name, label, server.address)
      .catch((error: unknown) =>
        app.notices.push("error", error instanceof Error ? error.message : String(error)),
      );
  };

  const move = (by: 1 | -1) => {
    if (column === "hosts") {
      // The way to add a host comes after the last of them.
      const places = [...hosts.map((listed) => listed.key), ADD];
      const from = adding ? places.length - 1 : hostAt;
      const to = places[(from + by + places.length) % places.length];
      if (to === ADD) setAdding(true);
      else if (to !== undefined) look(to);
      return;
    }
    if (stops.length === 0) return;
    setStopAt(stops[(marked + by + stops.length) % stops.length]?.key ?? null);
  };
  const accept = (agreed: boolean) => {
    if (column === "hosts") {
      if (adding) add(true);
      else setColumn("rows");
      return;
    }
    if (!at) return;
    if (at.kind === "row") openRow(at.row);
    else act(at.act, agreed, true);
  };
  const onKey = (event: KeyboardEvent<HTMLInputElement>) => {
    const command = event.ctrlKey || event.metaKey;
    switch (event.key) {
      case "ArrowDown":
        move(1);
        break;
      case "ArrowUp":
        move(-1);
        break;
      case "Tab":
        setAdding(false);
        setColumn(column === "hosts" ? "rows" : "hosts");
        break;
      case "Enter":
        accept(command);
        break;
      case "F2":
        if (column === "hosts") {
          if (host && !adding) startHostRename(host);
        } else if (at?.kind === "row") startRename(at.row);
        break;
      case "Delete":
        if (!command) {
          event.stopPropagation();
          return;
        }
        if (at?.kind === "row") take(at.row);
        break;
      case "Escape":
        close();
        break;
      default:
        // What is typed here is the filter, not a key for a terminal.
        event.stopPropagation();
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  };

  const tile = (listed: SelectorHost): ReactNode => (
    <span className="selector-tile" data-link={listed.link}>
      <Glyph name={listed.rex ? "server" : "monitor"} size={15} weight={1.8} />
    </span>
  );
  const why = host?.rex ? reason(host.rex) : null;
  const problem = host?.rex ? rex.problem(host.rex) : null;
  const heading = host ? (host.rex ? host.rex.address : "Workspaces of this computer") : "";

  return (
    <>
      <div className="menu-backdrop" onPointerDown={close} />
      <div
        ref={box}
        className="selector"
        role="dialog"
        aria-label="Hosts and sessions"
        data-animate={!keyboard || undefined}
        onBlur={(event) => {
          // Something else took the keyboard: a tab was changed to, a question asked.
          const to = event.relatedTarget;
          if (to instanceof Node && !event.currentTarget.contains(to)) dismiss();
        }}
        onMouseDown={(event) => {
          // A press on the selector leaves the keyboard in its field.
          if (!(event.target as Element).closest("input")) event.preventDefault();
        }}
      >
        <div className="selector-field">
          <SearchIcon />
          <input
            ref={field}
            className="selector-input"
            type="text"
            role="combobox"
            aria-label="Filter, or name something new"
            aria-expanded="true"
            aria-controls={column === "hosts" ? HOSTS_ID : ROWS_ID}
            aria-autocomplete="list"
            aria-activedescendant={
              column === "hosts"
                ? adding
                  ? ADD_ID
                  : hostAt >= 0 && renaming?.key !== host?.key
                    ? hostId(hostAt)
                    : undefined
                : at && at.key !== renaming?.key
                  ? optionId(marked)
                  : undefined
            }
            placeholder="Filter or create…"
            spellCheck={false}
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setStopAt(null);
            }}
            onKeyDown={onKey}
          />
        </div>
        <div className="selector-columns">
          <div id={HOSTS_ID} className="selector-hosts" role="listbox" aria-label="Hosts">
            {hosts.map((listed, index) => {
              if (renaming?.key === listed.key) {
                return (
                  <div key={listed.key} className="selector-host" data-renaming role="presentation">
                    {tile(listed)}
                    <RenameField
                      label="Host name"
                      value={renaming.value}
                      taken={labelTaken(renaming.value, servers?.hosts ?? [], listed.name) !== null}
                      set={(value) => setRenaming({ key: listed.key, value })}
                      save={() => saveHostRename(listed, renaming.value)}
                      cancel={stopRename}
                    />
                  </div>
                );
              }
              return (
                <div key={listed.key} className="selector-place" role="presentation">
                  <div
                    id={hostId(index)}
                    className="selector-host"
                    role="option"
                    aria-selected={listed.key === host?.key}
                    data-host={listed.key}
                    data-off={listed.link === "down" || undefined}
                    data-marked={(column === "hosts" && !adding && index === hostAt) || undefined}
                    onPointerMove={() => look(listed.key)}
                    onClick={() => {
                      look(listed.key);
                      setColumn("rows");
                    }}
                  >
                    {tile(listed)}
                    <span className="selector-text">
                      <span className="selector-name">{listed.name}</span>
                      <span className="selector-line">{listed.line}</span>
                    </span>
                    {listed.link === "busy" && <LinkDot link="busy" className="switcher-dot" />}
                  </div>
                  {renames(listed) && (
                    <span className="selector-tools">
                      <button
                        type="button"
                        tabIndex={-1}
                        aria-label={`Rename ${listed.name}`}
                        title="Rename (F2)"
                        onClick={() => startHostRename(listed)}
                      >
                        <Glyph name="pencil" size={13} />
                      </button>
                    </span>
                  )}
                </div>
              );
            })}
            {hosts.length === 0 && <p className="selector-note">Nothing matches.</p>}
            <div className="selector-add" role="presentation">
              <div className="selector-separator" role="presentation" />
              <div
                id={ADD_ID}
                className="selector-act"
                data-act="command"
                role="option"
                aria-selected={adding}
                data-marked={(column === "hosts" && adding) || undefined}
                onPointerMove={() => {
                  setColumn("hosts");
                  setAdding(true);
                }}
                onClick={() => add(false)}
              >
                <span className="selector-act-icon">
                  <Glyph name="plus" size={13} weight={2.2} />
                </span>
                <span className="selector-act-label">Add connection…</span>
              </div>
            </div>
          </div>
          {host && (
            <div className="selector-side">
              <div className="selector-heading">{heading}</div>
              {why !== null && (
                <p className="selector-why" role="status">
                  {why}
                </p>
              )}
              {problem !== null && (
                <p className="selector-why" data-problem role="alert">
                  {problem}
                </p>
              )}
              {host.key === THIS_PC && workspaces === null && (
                <p className="selector-note">Loading…</p>
              )}
              {host.link === "up" &&
                rows.length === 0 &&
                query.trim() === "" &&
                (host.rex !== null || workspaces !== null) && (
                  <p className="selector-note">
                    {host.rex ? `No sessions on ${host.name} yet.` : "No workspaces yet."}
                  </p>
                )}
              <div
                id={ROWS_ID}
                className="selector-rows"
                role="listbox"
                aria-label={host.rex ? `Sessions of ${host.name}` : "Workspaces"}
              >
                {stops.map((stop, index) => {
                  const chosen = column === "rows" && index === marked;
                  const common = {
                    id: optionId(index),
                    role: "option" as const,
                    "aria-selected": index === marked,
                    "data-marked": chosen || undefined,
                    onPointerMove: () => {
                      if (index !== marked || column !== "rows") {
                        setColumn("rows");
                        setStopAt(stop.key);
                      }
                    },
                  };
                  if (stop.kind === "act") {
                    const { act: what } = stop;
                    const begins = stops[index - 1]?.kind !== "act";
                    return (
                      <div key={stop.key} role="presentation">
                        {begins && <div className="selector-separator" role="presentation" />}
                        <div
                          {...common}
                          className="selector-act"
                          data-act={what.kind}
                          onClick={() => act(what, true)}
                        >
                          <span className="selector-act-icon">
                            <Glyph name={actGlyph(what)} size={13} weight={2.2} />
                          </span>
                          <span className="selector-act-label">{what.label}</span>
                          {what.kind === "connect" && what.agree && (
                            <KeyCaps label={agreeKey} className="connect-keys" />
                          )}
                        </div>
                      </div>
                    );
                  }
                  const { row } = stop;
                  if (renaming?.key === row.key) {
                    return (
                      <div key={row.key} className="selector-row" data-renaming role="presentation">
                        <span className="selector-mark" aria-hidden="true" />
                        <RenameField
                          label={host.rex ? "Session name" : "Workspace name"}
                          value={renaming.value}
                          taken={rows.some(
                            (other) =>
                              other.key !== row.key &&
                              other.name.toLowerCase() === renaming.value.trim().toLowerCase(),
                          )}
                          set={(value) => setRenaming({ key: row.key, value })}
                          save={() => saveRename(row, renaming.value)}
                          cancel={stopRename}
                        />
                      </div>
                    );
                  }
                  return (
                    <div key={row.key} className="selector-place" role="presentation">
                      <div
                        {...common}
                        className="selector-row"
                        data-row={row.key}
                        data-on={row.on || undefined}
                        onClick={() => openRow(row)}
                      >
                        <span className="selector-mark" aria-hidden="true">
                          {row.on && <Glyph name="check" size={13} weight={2.6} />}
                        </span>
                        <span className="selector-name">{row.name}</span>
                        <span className="selector-right">{row.on ? "Attached" : row.hint}</span>
                      </div>
                      {(row.renames || row.takes !== null) && (
                        <span className="selector-tools">
                          {row.renames && (
                            <button
                              type="button"
                              tabIndex={-1}
                              aria-label={`Rename ${row.name}`}
                              title="Rename (F2)"
                              onClick={() => startRename(row)}
                            >
                              <Glyph name="pencil" size={13} />
                            </button>
                          )}
                          {row.takes !== null && (
                            <button
                              type="button"
                              tabIndex={-1}
                              data-danger
                              aria-label={`${row.takes === "end" ? "End" : "Delete"} ${row.name}`}
                              title={`${row.takes === "end" ? "End session" : "Delete workspace"} (${mac ? "⌘⌦" : "Ctrl+Delete"})`}
                              onClick={() => take(row)}
                            >
                              <Glyph name="x" size={13} weight={2.2} />
                            </button>
                          )}
                        </span>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      </div>
    </>
  );
}

/** Whether a host is a server the user can call something else. */
function renames(host: SelectorHost): boolean {
  return host.rex !== null && host.rex.written && usable(host.rex);
}

/** The picture of something that can be done. */
function actGlyph(act: SelectorAct) {
  switch (act.kind) {
    case "connect":
      return act.agree ? "triangle-exclamation-point" : "refresh-cw";
    case "disconnect":
      return "x";
    case "edit":
      return "pencil";
    case "command":
      return act.command === "lume.workspace.new" ? "plus" : "folder";
    default:
      return "plus";
  }
}
