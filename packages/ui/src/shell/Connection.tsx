/**
 * The dialog that adds or changes a connection, over a window of terminals. A Rex server
 * that was added is shown in the selector, where it can be connected to; a machine
 * reached over SSH or a Linux distribution in the Connect palette, which lists the
 * profiles.
 */

import { useStore } from "zustand";

import { AddConnection, type ConnectionDone } from "../connections/AddConnection.tsx";
import { connectionDialog } from "../connections/store.ts";
import { useShell } from "./context.ts";
import { selectorDoor } from "./host-selector.ts";

export function Connection() {
  const app = useShell();
  const asked = useStore(connectionDialog, (state) => state.open);
  const servers = useStore(app.rex, (state) => state.list);
  const profiles = useStore(app.profiles, (state) => state.list);
  const platform = useStore(app.config, (state) => state.view?.platform ?? "windows");
  if (asked === null) return null;
  const done = (what: ConnectionDone) => {
    if (what.kind === "rex") selectorDoor.getState().open(what.label);
    else app.connect.getState().open(what.kind);
  };
  return (
    <AddConnection
      key={asked.opening}
      asked={asked}
      services={{
        client: app.client,
        hosts: servers?.hosts ?? [],
        profiles: profiles?.profiles ?? [],
        platform,
      }}
      close={() => {
        connectionDialog.getState().close();
        // The keyboard goes back to where the user was working.
        app.views.focus();
      }}
      done={done}
    />
  );
}
