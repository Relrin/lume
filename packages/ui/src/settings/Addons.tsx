/**
 * The plugins in the settings window: the page that manages them, and the sections a
 * plugin's own page shows after its settings.
 *
 * Managing them is turning each on or off, moving it earlier or later in the order
 * they load in, removing it, and installing one from a folder. Installing shows what
 * the package would be able to do before anything is installed, as `lume plugin
 * install` does, and offers the same choices: to replace the same plugin, or to
 * install another under a new id beside the one installed.
 *
 * A plugin's page then says what it may do, how it fares, and what it did lately. Its
 * words come from its manifest and its log, and are shown as text.
 */

import { useEffect, useId, useState, type ReactNode } from "react";
import { useStore } from "zustand";

import { Glyph } from "../icons/Glyph.tsx";
import { sentence } from "../shell/notices.ts";
import type { SettingsApp } from "./boot.ts";
import {
  installHeading,
  type AddonEntry,
  type AddonPermission,
  type InstallPreview,
  type InstallTaken,
} from "./addons.ts";
import { Button } from "./rows.tsx";

/** How often what is installed is asked for again while a page about it is on show, so
 *  that how a plugin fares and what it did keep up. */
const FRESH_MS = 5000;

/** Asks for the plugins again now, and every so often while the page is on show. */
function useFresh(app: SettingsApp, page: string): void {
  useEffect(() => {
    app.refreshAddons();
    const timer = setInterval(() => app.refreshAddons(), FRESH_MS);
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
function PermissionLines({ permissions }: { permissions: readonly AddonPermission[] }) {
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
export function addonStatus(addon: AddonEntry): { text: string; problem: boolean } | null {
  if (addon.modified) {
    return {
      text: "Its files have changed since it was installed, so it is not used. Install it again to use it.",
      problem: true,
    };
  }
  if (addon.missing) return { text: "Its folder is gone, so it is not used.", problem: true };
  if (!addon.enabled) return { text: "Turned off.", problem: false };
  if (!addon.state.ready) {
    return { text: `Not used: ${addon.state.reason ?? "it cannot be loaded"}`, problem: true };
  }
  if (addon.health.breaches > 0) {
    return {
      text: `Stopped ${addon.health.breaches === 1 ? "once" : `${addon.health.breaches} times`} in the last minute for going over its limits.`,
      problem: true,
    };
  }
  return null;
}

/** What an install depends on, in words. */
export function dependencyWords(addon: AddonEntry): string | null {
  if (addon.dependencies.length === 0) return null;
  const parts = addon.dependencies.map((dependency) => {
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
function AddonRow({
  app,
  addon,
  first,
  last,
}: {
  app: SettingsApp;
  addon: AddonEntry;
  first: boolean;
  last: boolean;
}) {
  const label = useId();
  const [removing, setRemoving] = useState(false);
  const act = (asked: Promise<unknown>) => {
    asked.then(
      () => app.refreshAddons(),
      (error: unknown) => app.notices.push("error", sentence(reason(error))),
    );
  };
  const status = addonStatus(addon);
  const dependencies = dependencyWords(addon);
  const under =
    addon.id === addon.manifestId ? addon.id : `${addon.manifestId}, installed as ${addon.id}`;
  return (
    <div className="settings-row addon-row" data-addon={addon.id}>
      <div className="settings-label">
        <div id={label}>
          {addon.name}
          <span className="settings-from">v{addon.version}</span>
        </div>
        <div>{under}</div>
        {dependencies !== null && <div>{dependencies}</div>}
        {status !== null && (
          <div className={status.problem ? "settings-problem" : "settings-note"}>{status.text}</div>
        )}
      </div>
      <div className="addon-actions">
        {removing ? (
          <>
            <span className="addon-ask">Remove {addon.name}?</span>
            <Button onPress={() => setRemoving(false)}>Keep</Button>
            <Button primary onPress={() => act(app.client.addonRemove(addon.id))}>
              Remove
            </Button>
          </>
        ) : (
          <>
            <button
              type="button"
              className="addon-move"
              aria-label={`Load ${addon.name} earlier`}
              disabled={first}
              onClick={() => act(app.client.addonMove(addon.id, true))}
            >
              <Glyph name="arrow-up" size={14} />
            </button>
            <button
              type="button"
              className="addon-move"
              aria-label={`Load ${addon.name} later`}
              disabled={last}
              onClick={() => act(app.client.addonMove(addon.id, false))}
            >
              <Glyph name="arrow-down" size={14} />
            </button>
            <Button onPress={() => setRemoving(true)}>Remove</Button>
            <button
              type="button"
              role="switch"
              className="settings-switch"
              aria-checked={addon.enabled}
              aria-labelledby={label}
              onClick={() => act(app.client.addonSetEnabled(addon.id, !addon.enabled))}
            />
          </>
        )}
      </div>
    </div>
  );
}

/** Where installing a package from a folder has got to. */
type Installing =
  | { readonly kind: "looking"; readonly folder: string }
  | { readonly kind: "preview"; readonly preview: InstallPreview }
  | {
      readonly kind: "taken";
      readonly folder: string;
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
  inspect: (folder: string, as: string | null) => void;
  done: () => void;
}) {
  const field = useId();
  const [as, setAs] = useState(installing.kind === "taken" ? installing.as : "");
  if (installing.kind === "looking") {
    return (
      <Group label="Install">
        <Line label="Looking at the package…" />
      </Group>
    );
  }
  if (installing.kind === "taken") {
    const { taken } = installing;
    return (
      <Group label="Install">
        <div className="settings-row addon-install">
          <div className="settings-label">
            <div>Another package is installed as {taken.local}</div>
            <div>
              It came from {taken.existing}. This one can be installed beside it under an id of its
              own.
            </div>
            <label className="addon-as" htmlFor={field}>
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
          <div className="addon-actions">
            <Button onPress={done}>Cancel</Button>
            <Button primary onPress={() => inspect(installing.folder, as.trim() || null)}>
              Continue
            </Button>
          </div>
        </div>
      </Group>
    );
  }
  const { preview } = installing;
  const install = () => {
    app.client.addonInstall(preview.token).then(
      () => {
        done();
        app.refreshAddons();
      },
      (error: unknown) => app.notices.push("error", sentence(reason(error))),
    );
  };
  const cancel = () => {
    app.client.addonCancel(preview.token).catch(() => undefined);
    done();
  };
  const replacing = preview.change !== "new";
  return (
    <Group label="Install">
      <div className="settings-row addon-install">
        <div className="settings-label">
          <div>{installHeading(preview)}</div>
          <div>
            {preview.local === preview.id
              ? preview.id
              : `${preview.id}, installed as ${preview.local}`}
          </div>
          {preview.description !== null && <div>{preview.description}</div>}
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
      <div className="settings-row addon-install-actions">
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
export function AddonsManager({ app }: { app: SettingsApp }) {
  const list = useStore(app.addons, (state) => state.list);
  const [installing, setInstalling] = useState<Installing | null>(null);
  useFresh(app, "manage");

  const inspect = (folder: string, as: string | null) => {
    setInstalling({ kind: "looking", folder });
    app.client.addonInspect(folder, as).then(
      (inspected) => {
        if ("taken" in inspected) {
          setInstalling({
            kind: "taken",
            folder,
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
  const pick = () => {
    app.pickFolder().then(
      (folder) => {
        if (folder !== null) inspect(folder, null);
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
        {list?.map((addon, index) => (
          <AddonRow
            key={addon.id}
            app={app}
            addon={addon}
            first={index === 0}
            last={index === list.length - 1}
          />
        ))}
      </Group>
    </>
  );
}

/** What a plugin's own page shows after its settings: what it may do, how it fares,
 *  and what it did lately. */
export function AddonSections({ app, id }: { app: SettingsApp; id: string }) {
  const addon = useStore(app.addons, (state) => state.list?.find((entry) => entry.id === id));
  useFresh(app, id);
  if (addon === undefined) return null;
  const status = addonStatus(addon);
  const state = !addon.enabled
    ? "Turned off"
    : !addon.state.ready || addon.modified || addon.missing
      ? "Not used"
      : addon.health.running
        ? "Running"
        : "Ready, not running";
  return (
    <>
      <Group label="Permissions">
        {addon.permissions.length === 0 && <Line label="It asks for no permissions." />}
        <PermissionLines permissions={addon.permissions} />
        {addon.later.length > 0 && (
          <Line label="It may ask you later to:">
            {addon.later.map((what, index) => (
              <div key={index}>{what}</div>
            ))}
          </Line>
        )}
      </Group>
      <Group label="Health">
        <Line label={state}>
          {status !== null && (
            <div className={status.problem ? "settings-problem" : "settings-note"}>
              {status.text}
            </div>
          )}
        </Line>
        <Line
          label={`Started ${addon.health.starts === 1 ? "once" : `${addon.health.starts} times`} since Lume started`}
        />
        {addon.health.lastProblem !== null && (
          <Line label="Last problem">
            <div className="addon-text">{addon.health.lastProblem}</div>
          </Line>
        )}
        {addon.health.warnings.map((warning, index) => (
          <Line key={index} label="Not as its manifest asks">
            <div className="addon-text">{warning}</div>
          </Line>
        ))}
      </Group>
      <Group label="Activity">
        {addon.audit.length === 0 ? (
          <Line label="It has used no permission lately." />
        ) : (
          <div className="settings-row">
            <ul className="addon-audit" aria-label="What it did lately">
              {addon.audit.map((entry, index) => (
                <li key={index} data-outcome={entry.outcome}>
                  <span className="addon-audit-time">
                    {entry.time.replace("T", " ").replace(/\.\d+Z$|Z$/, "")}
                  </span>
                  <span className="addon-audit-what">
                    {entry.outcome === "denied" ? "Refused" : "Allowed"} {entry.capability}
                    {entry.scope !== "" ? ` (${entry.scope})` : ""}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        )}
        {addon.log.length > 0 && (
          <div className="settings-row">
            <pre className="addon-log" aria-label="Its log">
              {addon.log.join("\n")}
            </pre>
          </div>
        )}
      </Group>
    </>
  );
}
