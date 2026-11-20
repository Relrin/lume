/**
 * The page of the settings window that lists the commands: every one there is, under
 * what it acts on, with the keys that run it now.
 *
 * The keys are the ones the configuration in use binds, so a key the user changed is
 * shown as they changed it, and the page is right again as soon as the configuration
 * is read again. The commands are asked for when the page is shown and whenever the
 * configuration changes: plugins bring commands, and come and go.
 */

import { useEffect, useMemo, useState } from "react";
import { useStore } from "zustand";

import { listCommands } from "../commands/groups.ts";
import type { CommandSpec } from "../core/types.ts";
import { sentence } from "../shell/notices.ts";
import type { SettingsApp } from "./boot.ts";

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** The keys of one command: each way to press it, or a dash when there is none. */
function Keys({ keys }: { keys: readonly string[] }) {
  if (keys.length === 0) {
    return (
      <span className="command-keys" data-none="true" aria-label="No key">
        —
      </span>
    );
  }
  return (
    <span className="command-keys">
      {keys.map((key) => (
        <kbd key={key} className="command-key">
          {key}
        </kbd>
      ))}
    </span>
  );
}

/** The list of commands, with a field that narrows it. */
export function CommandsPage({ app }: { app: SettingsApp }) {
  const view = useStore(app.config, (state) => state.view);
  const [commands, setCommands] = useState<readonly CommandSpec[] | null>(null);
  const [query, setQuery] = useState("");
  const revision = view?.revision;

  useEffect(() => {
    let current = true;
    app.client.commandsList().then(
      (list) => {
        if (current) setCommands(list);
      },
      (error: unknown) => {
        if (!current) return;
        setCommands((held) => held ?? []);
        app.notices.push("error", `The commands could not be read: ${sentence(reason(error))}`);
      },
    );
    return () => {
      current = false;
    };
  }, [app, revision]);

  const bindings = view?.keymap.bindings;
  const platform = view?.platform;
  const groups = useMemo(
    () =>
      commands && bindings && platform ? listCommands(commands, bindings, platform, query) : [],
    [commands, bindings, platform, query],
  );
  const count = groups.reduce((sum, group) => sum + group.commands.length, 0);

  return (
    <div className="commands-page">
      <input
        className="commands-filter"
        type="text"
        aria-label="Filter commands"
        placeholder="Filter by name, id or key"
        autoComplete="off"
        spellCheck={false}
        value={query}
        onChange={(event) => setQuery(event.target.value)}
      />
      {commands !== null && count === 0 && (
        <div className="settings-empty">
          {query.trim() === "" ? "There are no commands." : `No commands match “${query.trim()}”`}
        </div>
      )}
      {groups.map(({ group, commands: listed }) => (
        <section key={group} className="settings-group">
          <h2>
            {group}
            <span className="commands-count">{listed.length}</span>
          </h2>
          <div className="settings-rows">
            {listed.map((command) => (
              <div key={command.id} className="settings-row command-row">
                <div className="command-name">
                  <span className="command-title">
                    {command.title}
                    {command.plugin && <span className="command-from">{command.category}</span>}
                  </span>
                  <code className="command-id">{command.id}</code>
                </div>
                <Keys keys={command.keys} />
              </div>
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}
