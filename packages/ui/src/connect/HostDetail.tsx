/**
 * What the column beside the list says of a Rex server: how near it is and what it runs,
 * its sessions with the one that would be attached to marked, and what can be done with
 * it. A server that is not connected says why, and how to go on.
 */

import { useEffect, useRef } from "react";

import { Glyph } from "../icons/Glyph.tsx";
import { KeyCaps } from "../palette/KeyCaps.tsx";
import {
  canDisconnect,
  connected,
  nearness,
  next,
  onThisMachine,
  reason,
  sessionName,
  sessionOnShow,
  systemName,
} from "../rex/model.ts";
import type { HostEntry } from "./model.ts";

/** What a session's row in the column is called, for the field to point at. */
export function sessionOptionId(index: number): string {
  return `connect-session-${index}`;
}

/** What can be done with a server from the column. */
export interface HostActs {
  /** Puts a session on show. */
  attach(entry: HostEntry, session: string): void;
  newSession(entry: HostEntry): void;
  connect(entry: HostEntry, agreed: boolean): void;
  disconnect(entry: HostEntry): void;
  copy(entry: HostEntry): void;
  pin(entry: HostEntry): void;
}

/** A button of the column, which leaves the keyboard in the palette's field. */
function Act({
  label,
  keys,
  primary = false,
  act,
}: {
  label: string;
  keys?: string | undefined;
  primary?: boolean;
  act: () => void;
}) {
  return (
    <button
      type="button"
      className={primary ? "connect-primary" : "connect-secondary"}
      tabIndex={-1}
      onPointerDown={(event) => event.preventDefault()}
      onClick={act}
    >
      <span className="connect-act-label">{label}</span>
      {keys !== undefined && <KeyCaps label={keys} className="connect-keys" />}
    </button>
  );
}

/** The column for a Rex server. */
export function HostDetail({
  entry,
  backend,
  marked,
  inside,
  problem,
  agreeKey,
  mac,
  acts,
  mark,
}: {
  entry: HostEntry;
  /** Whose layout the window shows, if a server's. */
  backend: string | null;
  /** Which of its sessions is marked. */
  marked: number;
  /** Whether the arrow keys move among the sessions. */
  inside: boolean;
  /** What went wrong, besides how the server stands. */
  problem: string | null;
  /** The keys that agree to a connection without encryption. */
  agreeKey: string;
  mac: boolean;
  acts: HostActs;
  mark: (index: number) => void;
}) {
  const { host } = entry;
  const list = useRef<HTMLDivElement>(null);
  const state = host.state;
  const up = connected(host);
  const shown = sessionOnShow(host, backend);
  const chosen = host.sessions[marked];
  const why = reason(host);
  const step = next(host);
  useEffect(() => {
    list.current
      ?.querySelector(`#${sessionOptionId(marked)}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [marked]);
  const stats: [string, string, string | undefined][] = [
    ...(onThisMachine(host)
      ? []
      : [["Latency", up ? nearness(host) || "…" : "—", undefined] as [string, string, undefined]]),
    [
      "Server",
      state.kind === "connected" ? systemName(state.platform) : "—",
      state.kind === "connected" ? `Rex ${state.version} on ${state.platform}` : undefined,
    ],
    ["API rev", state.kind === "connected" ? String(state.api_revision) : "—", undefined],
  ];
  const key = (pc: string, apple: string) => (mac ? apple : pc);
  return (
    <section className="connect-detail" aria-label="Selection">
      <div className="connect-detail-top">
        <div className="connect-detail-head">
          <h2 className="connect-detail-name">{host.label}</h2>
          <span className="connect-detail-line">{host.address}</span>
        </div>
        <button
          type="button"
          className="connect-pin"
          aria-pressed={entry.pinned}
          aria-label={entry.pinned ? `Unpin ${host.label}` : `Pin ${host.label}`}
          title={entry.pinned ? "Unpin (Ctrl+D)" : "Pin to the top (Ctrl+D)"}
          tabIndex={-1}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => acts.pin(entry)}
        >
          <Glyph name="pin" size={14} />
        </button>
      </div>
      <dl className="connect-stats" data-count={stats.length}>
        {stats.map(([label, value, note]) => (
          <div key={label} title={note}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      {why !== null && (
        <p className="connect-call" role="status">
          {why}
        </p>
      )}
      {problem !== null && (
        <p className="connect-call" data-problem role="alert">
          {problem}
        </p>
      )}
      {up && (
        <div className="connect-sessions" data-inside={inside || undefined}>
          <h3 className="connect-sessions-title">Sessions</h3>
          {host.sessions.length === 0 && (
            <p className="connect-detail-about">None yet. A new one is a shell on the server.</p>
          )}
          {host.sessions.length > 0 && (
            <div
              ref={list}
              className="connect-sessions-list"
              role="listbox"
              aria-label={`Sessions of ${host.label}`}
            >
              {host.sessions.map((session, index) => (
                <div
                  key={session.id}
                  id={sessionOptionId(index)}
                  className="connect-session"
                  role="option"
                  aria-selected={index === marked}
                  onPointerDown={(event) => event.preventDefault()}
                  onClick={() => mark(index)}
                  onDoubleClick={() => acts.attach(entry, session.id)}
                >
                  <span className="connect-dot" data-state="ended" aria-hidden="true" />
                  <span className="connect-session-name">{sessionName(session)}</span>
                  {session.id === shown && <span className="connect-meta">Attached</span>}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
      {!up && <div className="connect-detail-spacer" />}
      <div className="connect-acts">
        {up && chosen && (
          <Act
            primary
            label={`Attach to ${sessionName(chosen)}`}
            keys="↵"
            act={() => acts.attach(entry, chosen.id)}
          />
        )}
        {up && (
          <Act
            primary={!chosen}
            label="New session"
            keys={chosen ? key("Ctrl+N", "⌃N") : "↵"}
            act={() => acts.newSession(entry)}
          />
        )}
        {step !== null && (
          <Act
            primary
            label={step.label}
            keys={step.kind === "agree" ? agreeKey : "↵"}
            act={() => acts.connect(entry, step.kind === "agree")}
          />
        )}
        <Act label="Copy address" keys={key("Ctrl+Shift+C", "⌃⇧C")} act={() => acts.copy(entry)} />
        {canDisconnect(host) && (
          <Act
            label={state.kind === "connecting" ? "Cancel" : "Disconnect"}
            act={() => acts.disconnect(entry)}
          />
        )}
      </div>
    </section>
  );
}
