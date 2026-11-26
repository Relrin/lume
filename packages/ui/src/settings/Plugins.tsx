/**
 * The plugins in the settings window: the page that manages them, and the sections a
 * plugin's own page shows after its settings.
 *
 * Managing them is turning each on or off, moving it earlier or later in the order
 * they load in, removing it, and installing one from a folder or from a repository,
 * which is fetched with git. Installing shows what the package would be able to do
 * before anything is installed, as `lume plugin install` does, and offers the same
 * choices: to replace the same plugin, or to install another under a new id beside the
 * one installed. A plugin that came from a repository can be looked for a newer one of.
 *
 * A plugin's page then says what it may do, how it fares, and what it did lately. Its
 * words come from its manifest and its log, and are shown as text.
 */

import { useEffect, useId, useState, type ReactNode } from "react";
import { useStore } from "zustand";

import { Glyph } from "../icons/Glyph.tsx";
import { DevBadge } from "../plugins/DevBadge.tsx";
import { sentence } from "../shell/notices.ts";
import type { SettingsApp } from "./boot.ts";
import {
  installHeading,
  sourceWords,
  type InstalledPlugin,
  type PluginMachine,
  type PluginPermission,
  type InstallFrom,
  type InstallPreview,
  type InstallTaken,
} from "./plugins.ts";
import { Button } from "./rows.tsx";

/** How often what is installed is asked for again while a page about it is on show, so
 *  that how a plugin fares and what it did keep up. */
const FRESH_MS = 5000;

/** Asks for the plugins again now, and every so often while the page is on show. */
function useFresh(app: SettingsApp, page: string): void {
  useEffect(() => {
    app.refreshPlugins();
    const timer = setInterval(() => app.refreshPlugins(), FRESH_MS);
    return () => clearInterval(timer);
  }, [app, page]);
}

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** A heading over a box of rows, as the window's own groups are drawn. */
function Group({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section className="settings-group">
      <h2>{label}</h2>
      <div className="settings-rows">{children}</div>
    </section>
  );
}

/** A row of words: a label, and what is said of it. */
function Line({ label, children }: { label: string; children?: ReactNode }) {
  return (
    <div className="settings-row">
      <div className="settings-label">
        <div>{label}</div>
        {children}
      </div>
    </div>
  );
}

/** What a plugin can do, each in words with its warning. */
function PermissionLines({ permissions }: { permissions: readonly PluginPermission[] }) {
  return (
    <>
      {permissions.map((permission, index) => (
        <Line key={index} label={permission.what}>
          {permission.warning !== null && (
            <div className="settings-problem">{permission.warning}</div>
          )}
        </Line>
      ))}
    </>
  );
}

/** How an install fares, in a few words, where there is something to say. */
export function pluginStatus(plugin: InstalledPlugin): { text: string; problem: boolean } | null {
  if (plugin.modified) {
    return {
      text: "Its files have changed since it was installed, so it is not used. Install it again to use it.",
      problem: true,
    };
  }
  if (plugin.missing) return { text: "Its folder is gone, so it is not used.", problem: true };
  if (!plugin.enabled) return { text: "Turned off.", problem: false };
  if (!plugin.state.ready) {
    return { text: `Not used: ${plugin.state.reason ?? "it cannot be loaded"}`, problem: true };
  }
  if (plugin.health.breaches > 0) {
    return {
      text: `Stopped ${plugin.health.breaches === 1 ? "once" : `${plugin.health.breaches} times`} in the last minute for going over its limits.`,
      problem: true,
    };
  }
  return null;
}

/** What an install depends on, in words. */
export function dependencyWords(plugin: InstalledPlugin): string | null {
  if (plugin.dependencies.length === 0) return null;
  const parts = plugin.dependencies.map((dependency) => {
    if (dependency.bound === null) {
      return dependency.optional
        ? `can use ${dependency.id} (not installed)`
        : `needs ${dependency.id} (not installed)`;
    }
    return dependency.bound === dependency.id
      ? `uses ${dependency.id}`
      : `uses ${dependency.id} as installed under ${dependency.bound}`;
  });
  const said = parts.join(", ");
  return said.charAt(0).toUpperCase() + said.slice(1) + ".";
}

/** One installed plugin, with what can be done to it. */
function PluginRow({
  app,
  plugin,
  first,
  last,
  update,
}: {
  app: SettingsApp;
  plugin: InstalledPlugin;
  first: boolean;
  last: boolean;
  /** Looks for a newer one where it came from. */
  update: () => void;
}) {
  const label = useId();
  const [removing, setRemoving] = useState(false);
  const from = sourceWords(plugin);
  const act = (asked: Promise<unknown>) => {
    asked.then(
      () => app.refreshPlugins(),
      (error: unknown) => app.notices.push("error", sentence(reason(error))),
    );
  };
  const status = pluginStatus(plugin);
  const dependencies = dependencyWords(plugin);
  const under =
    plugin.id === plugin.manifestId ? plugin.id : `${plugin.manifestId}, installed as ${plugin.id}`;
  return (
    <div className="settings-row plugin-row" data-plugin={plugin.id}>
      <div className="settings-label">
        <div id={label}>
          {plugin.name}
          <span className="settings-from">v{plugin.version}</span>
          {plugin.dev && <DevBadge />}
        </div>
        <div>{under}</div>
        {from !== null && <div className="plugin-text">{from}</div>}
        {dependencies !== null && <div>{dependencies}</div>}
        {status !== null && (
          <div className={status.problem ? "settings-problem" : "settings-note"}>{status.text}</div>
        )}
      </div>
      <div className="plugin-actions">
        {removing ? (
          <>
            <span className="plugin-ask">Remove {plugin.name}?</span>
            <Button onPress={() => setRemoving(false)}>Keep</Button>
            <Button primary onPress={() => act(app.client.pluginRemove(plugin.id))}>
              Remove
            </Button>
          </>
        ) : (
          <>
            <button
              type="button"
              className="plugin-move"
              aria-label={`Load ${plugin.name} earlier`}
              disabled={first}
              onClick={() => act(app.client.pluginMove(plugin.id, true))}
            >
              <Glyph name="arrow-up" size={14} />
            </button>
            <button
              type="button"
              className="plugin-move"
              aria-label={`Load ${plugin.name} later`}
              disabled={last}
              onClick={() => act(app.client.pluginMove(plugin.id, false))}
            >
              <Glyph name="arrow-down" size={14} />
            </button>
            {plugin.sourceKind === "git" && <Button onPress={update}>Check for update</Button>}
            <Button onPress={() => setRemoving(true)}>Remove</Button>
            <button
              type="button"
              role="switch"
              className="settings-switch"
              aria-checked={plugin.enabled}
              aria-labelledby={label}
              onClick={() => act(app.client.pluginSetEnabled(plugin.id, !plugin.enabled))}
            />
          </>
        )}
      </div>
    </div>
  );
}

/** Where installing a package has got to. */
type Installing =
  | { readonly kind: "looking"; readonly from: InstallFrom }
  | { readonly kind: "preview"; readonly preview: InstallPreview }
  | {
      readonly kind: "taken";
      readonly from: InstallFrom;
      readonly taken: InstallTaken["taken"];
      readonly as: string;
    };

/** What installing a package would do, and the choice to go ahead. */
function InstallPanel({
  app,
  installing,
  inspect,
  done,
}: {
  app: SettingsApp;
  installing: Installing;
  inspect: (from: InstallFrom, as: string | null) => void;
  done: () => void;
}) {
  const field = useId();
  const [as, setAs] = useState(installing.kind === "taken" ? installing.as : "");
  if (installing.kind === "looking") {
    return (
      <Group label="Install">
        <Line
          label={
            installing.from.kind === "folder"
              ? "Looking at the package…"
              : "Fetching the repository…"
          }
        />
      </Group>
    );
  }
  if (installing.kind === "taken") {
    const { taken } = installing;
    return (
      <Group label="Install">
        <div className="settings-row plugin-install">
          <div className="settings-label">
            <div>Another package is installed as {taken.local}</div>
            <div>
              It came from {taken.existing}. This one can be installed beside it under an id of its
              own.
            </div>
            <label className="plugin-as" htmlFor={field}>
              Install it as
              <input
                id={field}
                type="text"
                spellCheck={false}
                autoComplete="off"
                value={as}
                onChange={(event) => setAs(event.target.value)}
              />
            </label>
          </div>
          <div className="plugin-actions">
            <Button onPress={done}>Cancel</Button>
            <Button primary onPress={() => inspect(installing.from, as.trim() || null)}>
              Continue
            </Button>
          </div>
        </div>
      </Group>
    );
  }
  const { preview } = installing;
  const install = () => {
    app.client.pluginInstall(preview.token).then(
      () => {
        done();
        app.refreshPlugins();
      },
      (error: unknown) => app.notices.push("error", sentence(reason(error))),
    );
  };
  const cancel = () => {
    app.client.pluginCancel(preview.token).catch(() => undefined);
    done();
  };
  const replacing = preview.change !== "new";
  return (
    <Group label="Install">
      <div className="settings-row plugin-install">
        <div className="settings-label">
          <div>{installHeading(preview)}</div>
          <div>
            {preview.local === preview.id
              ? preview.id
              : `${preview.id}, installed as ${preview.local}`}
          </div>
          {preview.description !== null && <div>{preview.description}</div>}
          <div className="plugin-text">
            From {preview.source}
            {preview.commit !== null && `, at commit ${preview.commit.slice(0, 12)}`}
          </div>
        </div>
      </div>
      <Line
        label={
          preview.permissions.length === 0
            ? replacing
              ? "It asks for nothing new."
              : "It asks for no permissions."
            : replacing
              ? "It will also be able to:"
              : "It will be able to:"
        }
      />
      <PermissionLines permissions={preview.permissions} />
      {preview.later.length > 0 && (
        <Line label="It may ask you later to:">
          {preview.later.map((what, index) => (
            <div key={index}>{what}</div>
          ))}
        </Line>
      )}
      {preview.warnings.map((warning, index) => (
        <Line key={`warning-${index}`} label="Before you install">
          <div className="settings-problem">{warning}</div>
        </Line>
      ))}
      <div className="settings-row plugin-install-actions">
        <span className="settings-head-space" />
        <Button onPress={cancel}>Cancel</Button>
        <Button primary onPress={install}>
          {replacing ? "Replace" : "Install"}
        </Button>
      </div>
    </Group>
  );
}

/** The page that manages the plugins. */
export function PluginsManager({ app }: { app: SettingsApp }) {
  const list = useStore(app.plugins, (state) => state.list);
  const [installing, setInstalling] = useState<Installing | null>(null);
  useFresh(app, "manage");

  const [git, setGit] = useState("");
  const [gitPath, setGitPath] = useState("");
  const gitField = useId();

  const inspect = (from: InstallFrom, as: string | null) => {
    setInstalling({ kind: "looking", from });
    const asked =
      from.kind === "folder"
        ? app.client.pluginInspect(from.folder, as)
        : from.kind === "git"
          ? app.client.pluginInspectGit(from.source, from.path, as)
          : app.client.pluginCheckUpdate(from.id);
    asked.then(
      (inspected) => {
        if ("current" in inspected) {
          setInstalling(null);
          app.notices.push(
            "info",
            `Version ${inspected.version} is what its repository has: there is nothing newer.`,
          );
        } else if ("taken" in inspected) {
          setInstalling({
            kind: "taken",
            from,
            taken: inspected.taken,
            as: inspected.taken.suggestion ?? "",
          });
        } else {
          setInstalling({ kind: "preview", preview: inspected });
        }
      },
      (error: unknown) => {
        setInstalling(null);
        app.notices.push("error", sentence(reason(error)));
      },
    );
  };
  const lookAtGit = () => {
    const source = git.trim();
    if (source === "") return;
    inspect({ kind: "git", source, path: gitPath.trim() || null }, null);
  };
  const pick = () => {
    app.pickFolder().then(
      (folder) => {
        if (folder !== null) inspect({ kind: "folder", folder }, null);
      },
      (error: unknown) => app.notices.push("error", sentence(reason(error))),
    );
  };

  return (
    <>
      {installing === null ? (
        <Group label="Install">
          <div className="settings-row">
            <div className="settings-label">
              <div>Install from a folder</div>
              <div>
                A plugin&apos;s folder, with its plugin.toml. You see what it would be able to do
                before anything is installed.
              </div>
            </div>
            <Button onPress={pick}>Install from folder…</Button>
          </div>
          <form
            className="settings-row"
            onSubmit={(event) => {
              event.preventDefault();
              lookAtGit();
            }}
          >
            <div className="settings-label">
              <div>
                <label htmlFor={gitField}>Install from git</label>
              </div>
              <div>
                A repository&apos;s address, or owner/repo for one on GitHub, with #branch or #tag
                after it if you want one; and the folder the plugin is in, if the repository holds
                more. It is fetched with the git on this machine, and the commit is recorded.
              </div>
              <div className="plugin-git">
                <input
                  id={gitField}
                  type="text"
                  spellCheck={false}
                  autoComplete="off"
                  placeholder="owner/repo#v1.0"
                  value={git}
                  onChange={(event) => setGit(event.target.value)}
                />
                <input
                  className="plugin-git-path"
                  type="text"
                  spellCheck={false}
                  autoComplete="off"
                  aria-label="The folder of the repository the plugin is in"
                  placeholder="folder (optional)"
                  value={gitPath}
                  onChange={(event) => setGitPath(event.target.value)}
                />
                <Button onPress={lookAtGit}>Look at it…</Button>
              </div>
            </div>
          </form>
        </Group>
      ) : (
        <InstallPanel
          key={installing.kind}
          app={app}
          installing={installing}
          inspect={inspect}
          done={() => setInstalling(null)}
        />
      )}
      <Group label="Installed">
        {list === null && <Line label="Looking at what is installed…" />}
        {list !== null && list.length === 0 && <Line label="No plugins are installed." />}
        {list?.map((plugin, index) => (
          <PluginRow
            key={plugin.id}
            app={app}
            plugin={plugin}
            first={index === 0}
            last={index === list.length - 1}
            update={() => inspect({ kind: "update", id: plugin.id }, null)}
          />
        ))}
      </Group>
    </>
  );
}

/** A machine a plugin may run programs on, with the way to take it back. */
function MachineRow({
  app,
  plugin,
  machine,
}: {
  app: SettingsApp;
  plugin: InstalledPlugin;
  machine: PluginMachine;
}) {
  const revoke = () => {
    app.client.pluginRevokeMachine(plugin.id, machine.id).then(
      () => app.refreshPlugins(),
      (error: unknown) => app.notices.push("error", sentence(reason(error))),
    );
  };
  return (
    <div className="settings-row" data-machine={machine.id}>
      <div className="settings-label">
        <div>{machine.label}</div>
        <div>It may run there, with any arguments: {machine.programs.join(", ")}</div>
      </div>
      <Button label={`Revoke what ${plugin.name} may run on ${machine.label}`} onPress={revoke}>
        Revoke
      </Button>
    </div>
  );
}

/** What a plugin's own page shows after its settings: what it may do, how it fares,
 *  and what it did lately. */
export function PluginSections({ app, id }: { app: SettingsApp; id: string }) {
  const plugin = useStore(app.plugins, (state) => state.list?.find((entry) => entry.id === id));
  useFresh(app, id);
  if (plugin === undefined) return null;
  const status = pluginStatus(plugin);
  const state = !plugin.enabled
    ? "Turned off"
    : !plugin.state.ready || plugin.modified || plugin.missing
      ? "Not used"
      : plugin.health.running
        ? "Running"
        : "Ready, not running";
  return (
    <>
      <Group label="Permissions">
        {plugin.permissions.length === 0 && <Line label="It asks for no permissions." />}
        <PermissionLines permissions={plugin.permissions} />
        {plugin.later.length > 0 && (
          <Line label="It may ask you later to:">
            {plugin.later.map((what, index) => (
              <div key={index}>{what}</div>
            ))}
          </Line>
        )}
      </Group>
      {plugin.machines.length > 0 && (
        <Group label="Other machines">
          {plugin.machines.map((machine) => (
            <MachineRow key={machine.id} app={app} plugin={plugin} machine={machine} />
          ))}
        </Group>
      )}
      <Group label="Health">
        <Line label={state}>
          {status !== null && (
            <div className={status.problem ? "settings-problem" : "settings-note"}>
              {status.text}
            </div>
          )}
        </Line>
        <Line
          label={`Started ${plugin.health.starts === 1 ? "once" : `${plugin.health.starts} times`} since Lume started`}
        />
        {plugin.health.lastProblem !== null && (
          <Line label="Last problem">
            <div className="plugin-text">{plugin.health.lastProblem}</div>
          </Line>
        )}
        {plugin.health.warnings.map((warning, index) => (
          <Line key={index} label="Not as its manifest asks">
            <div className="plugin-text">{warning}</div>
          </Line>
        ))}
      </Group>
      <Group label="Activity">
        {plugin.audit.length === 0 ? (
          <Line label="It has used no permission lately." />
        ) : (
          <div className="settings-row">
            <ul className="plugin-audit" aria-label="What it did lately">
              {plugin.audit.map((entry, index) => (
                <li key={index} data-outcome={entry.outcome}>
                  <span className="plugin-audit-time">
                    {entry.time.replace("T", " ").replace(/\.\d+Z$|Z$/, "")}
                  </span>
                  <span className="plugin-audit-what">
                    {entry.outcome === "denied" ? "Refused" : "Allowed"} {entry.capability}
                    {entry.scope !== "" ? ` (${entry.scope})` : ""}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        )}
        {plugin.log.length > 0 && (
          <div className="settings-row">
            <pre className="plugin-log" aria-label="Its log">
              {plugin.log.join("\n")}
            </pre>
          </div>
        )}
      </Group>
    </>
  );
}
