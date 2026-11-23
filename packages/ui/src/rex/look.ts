/**
 * Listing Rex servers and connecting to them, for whatever of the page is on show and
 * does so.
 */

import { useEffect, useState } from "react";

import type { RexHost } from "../core/types.ts";
import { useShell } from "../shell/context.ts";
import { sentence } from "../shell/notices.ts";
import { CommandError } from "../transport/types.ts";
import { REX_LOOK_EVERY_MS } from "./tuning.ts";

/**
 * Has the core look at the servers while `on`: whether one runs on this machine, how
 * near each connected one is, what sessions they have.
 */
export function useRexLook(on = true): void {
  const app = useShell();
  useEffect(() => {
    if (!on) return undefined;
    const look = () => {
      app.client.rexHostsLook().then(
        (list) => void app.rex.getState().set(list),
        // What is known is listed still.
        () => undefined,
      );
    };
    look();
    const timer = setInterval(look, REX_LOOK_EVERY_MS);
    return () => clearInterval(timer);
  }, [app, on]);
}

/** What a page does with a server while whatever lists it stays on show. */
export interface RexActions {
  /**
   * Connects to a server. `agreed` is whether the user agreed to a connection that is
   * not encrypted: they are told of it here, and asked nowhere else.
   */
  connect(host: RexHost, agreed: boolean): void;
  disconnect(host: RexHost): void;
  /** Calls a session something else. */
  rename(host: RexHost, session: string, label: string): void;
  /** What went wrong that the server's own standing does not say. */
  problem(host: RexHost): string | null;
}

/** The codes of a connect that was given up, which the user did themselves. */
const GIVEN_UP: readonly string[] = ["disconnected"];

/** Carries out what a page does with servers, and keeps what went wrong. */
export function useRexActions(): RexActions {
  const app = useShell();
  const [problems, setProblems] = useState<ReadonlyMap<string, string>>(() => new Map());
  const run = (
    command: string,
    host: RexHost,
    args: Readonly<Record<string, unknown>>,
    unsaid: readonly string[] = [],
  ) => {
    setProblems((held) => {
      const next = new Map(held);
      next.delete(host.id);
      return next;
    });
    app.client.commandRun(command, { host: host.id, ...args }).catch((error: unknown) => {
      if (error instanceof CommandError && unsaid.includes(error.code)) return;
      const message = sentence(error instanceof Error ? error.message : String(error));
      setProblems((held) => new Map(held).set(host.id, message));
    });
  };
  return {
    connect: (host, agreed) => run("lume.rex.connect", host, { confirmed: agreed }, GIVEN_UP),
    disconnect: (host) => run("lume.rex.disconnect", host, {}),
    rename: (host, session, label) => run("lume.rex.rename-session", host, { session, label }),
    // Why a connection was not made is how the server stands.
    problem: (host) => (host.state.kind === "failed" ? null : (problems.get(host.id) ?? null)),
  };
}
