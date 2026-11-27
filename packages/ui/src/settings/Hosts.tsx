/**
 * The page about the machines sessions can be opened on: the Rex servers, and the
 * profiles reached over SSH or opening a Linux distribution that the user added, each
 * with a way to change it and to take it out, and the way to add another.
 *
 * A server's row says how it stands. Nothing is connected to from here: this window
 * shows no session, and cannot be asked what connecting may ask.
 */

import { useId, useRef, useState, type ReactNode } from "react";
import { useStore } from "zustand";

import { describeRun } from "../connect/model.ts";
import { AddConnection } from "../connections/AddConnection.tsx";
import { ownSshProfiles, ownWslProfiles } from "../connections/model.ts";
import { connectionDialog } from "../connections/store.ts";
import type { RexHost } from "../core/types.ts";
import { LinkDot } from "../rex/LinkDot.tsx";
import { linkOf, standing } from "../rex/model.ts";
import { sentence } from "../shell/notices.ts";
import type { SettingsApp } from "./boot.ts";
import { Button } from "./rows.tsx";

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** One machine: what it is called and how it is reached, and what can be done with it. */
function Row({
  name,
  line,
  dot,
  badge,
  on = false,
  edit,
  remove,
}: {
  name: string;
  line: string;
  dot: ReactNode;
  badge: string;
  /** Whether the badge says that it is connected. */
  on?: boolean;
  edit?: (keyboard: boolean) => void;
  remove?: () => void;
}) {
  const label = useId();
  const [removing, setRemoving] = useState(false);
  return (
    <div className="settings-row host-row" aria-labelledby={label}>
      <span className="host-dot">{dot}</span>
      <div className="settings-label">
        <div id={label}>{name}</div>
        <div>{line}</div>
      </div>
      <div className="plugin-actions">
        {removing && remove ? (
          <>
            <span className="plugin-ask">Remove {name}?</span>
            <Button onPress={() => setRemoving(false)}>Keep</Button>
            <Button primary onPress={remove}>
              Remove
            </Button>
          </>
        ) : (
          <>
            <span className="host-badge" data-on={on || undefined}>
              {badge}
            </span>
            {edit && <Button onPress={() => edit(false)}>Edit</Button>}
            {remove && <Button onPress={() => setRemoving(true)}>Remove</Button>}
          </>
        )}
      </div>
    </div>
  );
}

/** How a server stands, in the word of its badge. */
function badgeOf(host: RexHost): string {
  switch (host.state.kind) {
    case "connected":
      return "Connected";
    case "idle":
      return "Idle";
    default:
      return standing(host);
  }
}

export function HostsPage({ app }: { app: SettingsApp }) {
  const servers = useStore(app.rex, (state) => state.list);
  const profiles = useStore(app.lists, (state) => state.profiles);
  const act = (asked: Promise<unknown>) => {
    asked.catch((error: unknown) => app.notices.push("error", sentence(reason(error))));
  };
  const hosts = servers?.hosts ?? [];
  return (
    <section className="settings-group">
      <h2>Hosts</h2>
      <div className="settings-rows">
        {hosts.map((host) => (
          <Row
            key={host.id}
            name={host.label}
            line={host.local && !host.written ? "This computer" : host.address}
            dot={<LinkDot link={linkOf(host)} className="switcher-dot" />}
            badge={badgeOf(host)}
            on={host.state.kind === "connected"}
            {...(host.written
              ? {
                  edit: (keyboard: boolean) =>
                    connectionDialog
                      .getState()
                      .edit({ kind: "rex", label: host.label, address: host.address }, keyboard),
                  remove: () => act(app.client.rexHostRemove(host.label)),
                }
              : {})}
          />
        ))}
        {ownSshProfiles(profiles).map((profile) => (
          <Row
            key={profile.id}
            name={profile.name}
            line={describeRun(profile)}
            dot={<LinkDot link="down" className="switcher-dot" />}
            badge="SSH"
            edit={(keyboard) =>
              connectionDialog.getState().edit({ kind: "ssh", profile }, keyboard)
            }
            remove={() => act(app.client.profileRemove(profile.id))}
          />
        ))}
        {ownWslProfiles(profiles).map((profile) => (
          <Row
            key={profile.id}
            name={profile.name}
            line={describeRun(profile) + (profile.cwd ? ` · ${profile.cwd}` : "")}
            dot={<LinkDot link="down" className="switcher-dot" />}
            badge="WSL"
            edit={(keyboard) =>
              connectionDialog.getState().edit({ kind: "wsl", profile }, keyboard)
            }
            remove={() => act(app.client.profileRemove(profile.id))}
          />
        ))}
        <div className="settings-row">
          <div className="settings-label">
            <div>Add a host</div>
            <div>A Rex server, SSH or WSL</div>
          </div>
          <Button primary onPress={() => connectionDialog.getState().add("rex", false)}>
            Add host…
          </Button>
        </div>
      </div>
    </section>
  );
}

/** The dialog over the settings window, while it is asked for. */
export function SettingsConnection({ app }: { app: SettingsApp }) {
  const asked = useStore(connectionDialog, (state) => state.open);
  const servers = useStore(app.rex, (state) => state.list);
  const profiles = useStore(app.lists, (state) => state.profiles);
  const platform = useStore(app.config, (state) => state.view?.platform ?? "windows");
  // What had the keyboard, for when the dialog is put away.
  const before = useRef<{ opening: number; element: Element | null } | null>(null);
  if (asked === null) return null;
  if (before.current?.opening !== asked.opening) {
    before.current = { opening: asked.opening, element: document.activeElement };
  }
  return (
    <AddConnection
      key={asked.opening}
      asked={asked}
      services={{ client: app.client, hosts: servers?.hosts ?? [], profiles, platform }}
      close={() => {
        connectionDialog.getState().close();
        const element = before.current?.element;
        if (element instanceof HTMLElement) element.focus();
      }}
      done={() => undefined}
    />
  );
}
