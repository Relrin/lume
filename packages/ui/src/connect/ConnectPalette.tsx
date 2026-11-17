/**
 * The palette that opens a terminal from a profile: a local shell, a Linux distribution,
 * another machine over SSH.
 *
 * It opens over the window with the keyboard in its field. Typing narrows the list, the
 * filters along the field show one kind of profile, the arrow keys move through the
 * list, and the column beside it says what the marked profile will run and where it
 * comes from. Enter opens the profile in a new tab and Ctrl+Enter beside the pane that
 * has the keyboard; Escape puts the palette away.
 */

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useStore } from "zustand";

import { useShell } from "../shell/context.ts";
import { SearchIcon } from "../shell/icons.tsx";
import { FileIcon, KindIcon, RefreshIcon } from "./icons.tsx";
import {
  CONNECT_FILTERS,
  connectGroups,
  connectRows,
  describeRun,
  openCommand,
  sourceOf,
  stepFilter,
  type ConnectAction,
  type ConnectEntry,
  type ConnectFilter,
  type ConnectRow,
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

/** The column that says what the marked profile, or action, is. */
function Detail({
  row,
  splitKey,
  open,
  act,
}: {
  row: ConnectRow | undefined;
  splitKey: string;
  open: (entry: ConnectEntry, how: "tab" | "split") => void;
  act: (action: ConnectAction) => void;
}) {
  if (!row) {
    return (
      <section className="connect-detail" aria-label="Selection">
        <p className="connect-detail-empty">Nothing is marked.</p>
      </section>
    );
  }
  if (row.type === "action") {
    return (
      <section className="connect-detail" aria-label="Selection">
        <h2 className="connect-detail-name">{row.action.label}</h2>
        <p className="connect-detail-about">{row.action.about}</p>
        <div className="connect-detail-spacer" />
        <div className="connect-detail-buttons">
          <button
            type="button"
            className="connect-primary"
            tabIndex={-1}
            onPointerDown={(event) => event.preventDefault()}
            onClick={() => act(row.action)}
          >
            {row.action.label}
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
  return (
    <section className="connect-detail" aria-label="Selection">
      <div className="connect-detail-head">
        <h2 className="connect-detail-name">{profile.name}</h2>
        <span className="connect-detail-line">{entry.line}</span>
      </div>
      <dl className="connect-facts">
        {facts.map(([label, value]) => (
          <div key={label} className="connect-fact">
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      <div className="connect-detail-spacer" />
      <div className="connect-detail-buttons">
        <button
          type="button"
          className="connect-primary"
          tabIndex={-1}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => open(entry, "tab")}
        >
          Open in new tab
        </button>
        <button
          type="button"
          className="connect-secondary"
          tabIndex={-1}
          title={`Split right (${splitKey})`}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => open(entry, "split")}
        >
          Split right
        </button>
      </div>
    </section>
  );
}

function ConnectDialog() {
  const app = useShell();
  const list = useStore(app.profiles, (state) => state.list);
  const platform = useStore(app.config, (state) => state.view?.platform);
  const field = useRef<HTMLInputElement>(null);
  const results = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<ConnectFilter>("all");
  const [selected, setSelected] = useState(0);

  useEffect(() => {
    field.current?.focus();
  }, []);

  const groups = useMemo(() => connectGroups(list, query, filter), [list, query, filter]);
  const rows = useMemo(() => connectRows(groups), [groups]);
  const marked = Math.min(selected, Math.max(0, rows.length - 1));
  const splitKey = platform === "macos" ? "⌘ ↵" : "Ctrl ↵";

  useEffect(() => {
    results.current?.querySelector(`#${optionId(marked)}`)?.scrollIntoView({ block: "nearest" });
  }, [marked, rows]);

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
  const accept = (index: number, how: "tab" | "split") => {
    const row = rows[index];
    if (!row) return;
    if (row.type === "profile") open(row.entry, how);
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
  const option = (row: ConnectRow, children: ReactNode, className: string) => {
    const at = index++;
    return (
      <div
        key={row.type === "profile" ? `profile:${row.entry.profile.id}` : `action:${row.action.id}`}
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
                  filterBy(stepFilter(filter, event.shiftKey ? -1 : 1));
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
          <span className="connect-chips" role="group" aria-label="Show">
            {CONNECT_FILTERS.map((choice) => (
              <button
                key={choice.value}
                type="button"
                className="connect-chip"
                data-filter={choice.value}
                aria-pressed={filter === choice.value}
                tabIndex={-1}
                onPointerDown={(event) => event.preventDefault()}
                onClick={() => filterBy(choice.value)}
              >
                {choice.label}
              </button>
            ))}
          </span>
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
                        <span className="connect-tile" data-kind={entry.kind}>
                          <KindIcon kind={entry.kind} />
                        </span>
                        <span className="connect-row-text">
                          <span className="connect-row-name">
                            <MarkedName text={entry.profile.name} indices={entry.indices} />
                          </span>
                          <span className="connect-row-line" data-kind={entry.kind}>
                            {entry.line}
                          </span>
                        </span>
                        {entry.isDefault && <span className="connect-badge">Default</span>}
                      </>,
                      "connect-row",
                    ),
                  )}
                </div>
              ))}
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
            {profileCount === 0 && (
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
          <Detail row={rows[marked]} splitKey={splitKey} open={open} act={act} />
        </div>
        <div className="connect-footer">
          <span>
            <span className="connect-key">↑↓</span> move
          </span>
          <span>
            <span className="connect-key">↵</span> open in new tab
          </span>
          <span>
            <span className="connect-key">{splitKey}</span> split right
          </span>
          <span>
            <span className="connect-key">Tab</span> filter
          </span>
          <span className="connect-footer-spacer" />
          <span>
            <span className="connect-key">Esc</span> close
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
