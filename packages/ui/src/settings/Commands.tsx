/**
 * The keybindings page of the settings window: every command there is, under what it
 * acts on, with the keys that run it now, and the way to change them.
 *
 * A key's colour says where it comes from: the application's default, one the user
 * set, one shared with another command, or a default the user unbound, struck
 * through. Pressing a key records another in its place; the plus beside a command
 * records one more. Whatever is changed is written to `[keybindings]` in the user's
 * file at once, and can be taken back from the note that says so.
 *
 * The commands are asked for when the page is shown and whenever the configuration
 * changes: plugins bring commands, and come and go.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "zustand";

import type { CommandSpec } from "../core/types.ts";
import { Glyph } from "../icons/Glyph.tsx";
import { chordOf } from "../keys/chord.ts";
import { keyCaps } from "../palette/KeyCaps.tsx";
import { SearchIcon } from "../shell/icons.tsx";
import { sentence } from "../shell/notices.ts";
import { currentPlatform, KeyLayout } from "../terminal/keyboard.ts";
import type { SettingsApp } from "./boot.ts";
import {
  addKey,
  KEY_FILTERS,
  keyCommands,
  keyMatches,
  keptBy,
  keysLabel,
  othersOn,
  replaceKey,
  resetAll,
  resetCommand,
  restoreKey,
  unbindKey,
  undoOf,
  type BoundKey,
  type KeyCommand,
  type KeyContext,
  type KeyFilter,
  type KeyWrite,
  type Taking,
} from "./keybindings.ts";

/** How long the note that says what changed stays, in milliseconds. */
const TOAST_MS = 4500;

/** How long a changed row is lit, in milliseconds. */
const FLASH_MS = 900;

/** How many chords a recorded key may have: a second press makes a chord of two. */
const CHORDS_MAX = 2;

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** A key being recorded: for which command, in place of which of its keys, and what so far. */
interface Recording {
  readonly id: string;
  readonly replace: BoundKey | null;
  readonly chords: readonly string[];
}

/** The note at the bottom: what changed, and how to take it back. */
interface Toast {
  readonly text: string;
  readonly undo: readonly KeyWrite[] | null;
}

/** Keys drawn as caps, coloured by where they come from. */
function Caps({ label, kind }: { label: string; kind?: "user" | "shared" | "off" }) {
  return (
    <span className="kb-keys" data-kind={kind} aria-label={label}>
      {keyCaps(label).map((chord, at) => (
        <span key={at} className="kb-chord" aria-hidden="true">
          {at > 0 && <span className="kb-then">then</span>}
          {chord.map((key, index) => (
            <kbd key={index}>{key}</kbd>
          ))}
        </span>
      ))}
    </span>
  );
}

function WarnIcon() {
  return <Glyph name="triangle-exclamation-point" size={12} />;
}

/** The list of commands and their keys, with what changes them. */
export function CommandsPage({ app }: { app: SettingsApp }) {
  const view = useStore(app.config, (state) => state.view);
  const [specs, setSpecs] = useState<readonly CommandSpec[] | null>(null);
  const [query, setQuery] = useState("");
  const [byKeys, setByKeys] = useState(false);
  const [filter, setFilter] = useState<KeyFilter>("all");
  const [recording, setRecording] = useState<Recording | null>(null);
  const [toast, setToast] = useState<Toast | null>(null);
  const [flash, setFlash] = useState<string | null>(null);
  const field = useRef<HTMLInputElement>(null);
  const revision = view?.revision;

  useEffect(() => {
    let current = true;
    app.client.commandsList().then(
      (list) => {
        if (current) setSpecs(list);
      },
      (error: unknown) => {
        if (!current) return;
        setSpecs((held) => held ?? []);
        app.notices.push("error", `The commands could not be read: ${sentence(reason(error))}`);
      },
    );
    return () => {
      current = false;
    };
  }, [app, revision]);

  const context: KeyContext | null = useMemo(
    () =>
      view
        ? {
            keymap: view.keymap,
            sources: view.sources,
            table: (view.config as { keybindings?: unknown }).keybindings,
            platform: view.platform,
          }
        : null,
    [view],
  );
  const commands = useMemo(
    () => (specs && context ? keyCommands(specs, context) : []),
    [specs, context],
  );
  const shown = commands.filter((command) => keptBy(filter, command));
  const found = shown.filter((command) =>
    byKeys && query !== ""
      ? command.keys.some((key) => key.label.toLowerCase().includes(query.toLowerCase()))
      : keyMatches(command, query),
  );
  const groups = [...new Set(found.map((command) => command.group))].map((group) => ({
    group,
    commands: found.filter((command) => command.group === group),
  }));
  const changed = commands.filter((command) => command.changed).length;

  // Timers are put away with the page.
  const toastTimer = useRef<number | null>(null);
  const flashTimer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (toastTimer.current !== null) window.clearTimeout(toastTimer.current);
      if (flashTimer.current !== null) window.clearTimeout(flashTimer.current);
    },
    [],
  );
  const say = (text: string, undo: readonly KeyWrite[] | null) => {
    if (toastTimer.current !== null) window.clearTimeout(toastTimer.current);
    setToast({ text, undo });
    toastTimer.current = window.setTimeout(() => setToast(null), TOAST_MS);
  };
  const light = (id: string) => {
    if (flashTimer.current !== null) window.clearTimeout(flashTimer.current);
    setFlash(id);
    flashTimer.current = window.setTimeout(() => setFlash(null), FLASH_MS);
  };

  /** Writes changes one after another, and says what went wrong if anything did. */
  const apply = async (writes: readonly KeyWrite[]): Promise<boolean> => {
    try {
      for (const write of writes) {
        if (write.op === "set") await app.client.configSet(write.path, write.value);
        else await app.client.configUnset(write.path);
      }
      return true;
    } catch (error: unknown) {
      app.notices.push("error", `The keys could not be changed: ${sentence(reason(error))}`);
      return false;
    }
  };
  const change = (writes: readonly KeyWrite[], done: string | null, id: string | null) => {
    if (!context || writes.length === 0) return;
    const undo = undoOf(writes, context);
    void apply(writes).then((made) => {
      if (!made) return;
      if (id !== null) light(id);
      if (done !== null) say(done, undo);
    });
  };

  const record = (id: string, replace: BoundKey | null) =>
    setRecording({ id, replace, chords: [] });

  /** Ends recording: keeps what was pressed (`taking` says how), or nothing. */
  const finish = (taking: Taking | null) => {
    const held = recording;
    setRecording(null);
    if (!held || taking === null || held.chords.length === 0 || !context) return;
    const command = commands.find((candidate) => candidate.id === held.id);
    if (!command) return;
    const keys = held.chords.join(" ");
    const writes = held.replace
      ? replaceKey(command, held.replace, keys, taking, context)
      : addKey(command, keys, taking, context);
    change(writes, null, command.id);
  };

  // While a key is recorded, every key pressed is the recorder's, and the window's own
  // keys (Escape, the key that closes it) wait.
  const platform = view?.platform ?? "windows";
  const state = useRef({ recording, byKeys, finish, platform });
  state.current = { recording, byKeys, finish, platform };
  useEffect(() => {
    const mapping = { platform: currentPlatform(), layout: new KeyLayout() };
    const onKey = (event: KeyboardEvent) => {
      const held = state.current.recording;
      const bare = !event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey;
      if (held) {
        event.preventDefault();
        event.stopPropagation();
        if (event.key === "Escape" && bare) return state.current.finish(null);
        if (event.key === "Enter" && bare && held.chords.length > 0) {
          return state.current.finish("keep");
        }
        if (event.key === "Backspace" && bare && held.chords.length > 0) {
          return setRecording({ ...held, chords: [] });
        }
        const chord = chordOf(event, mapping);
        if (chord === null) return;
        setRecording({
          ...held,
          chords: held.chords.length >= CHORDS_MAX ? [chord] : [...held.chords, chord],
        });
        return;
      }
      // Searching by keys: the field shows the keys pressed in it.
      if (state.current.byKeys && event.target === field.current) {
        event.preventDefault();
        event.stopPropagation();
        if (event.key === "Escape" && bare) return setByKeys(false);
        if (event.key === "Backspace" && bare) return setQuery("");
        const chord = chordOf(event, mapping);
        if (chord !== null) setQuery(keysLabel(chord, state.current.platform));
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  if (!context) return null;
  const recordedKeys = recording ? recording.chords.join(" ") : "";
  const recordedLabel = recordedKeys === "" ? "" : keysLabel(recordedKeys, context.platform);
  const clashing =
    recording && recordedKeys !== "" ? othersOn(commands, recording.id, recordedKeys) : [];

  const recorder = (
    <span className="kb-recorder" role="status" aria-live="polite">
      {recordedLabel === "" ? (
        <span className="kb-recorder-wait">Press keys…</span>
      ) : (
        <Caps label={recordedLabel} />
      )}
      <span className="kb-caret" />
    </span>
  );

  const row = (command: KeyCommand) => {
    const isRecording = recording?.id === command.id;
    return (
      <div key={command.id} className="kb-entry">
        <div
          className="kb-row"
          data-changed={command.changed || undefined}
          data-recording={isRecording || undefined}
          data-flash={flash === command.id || undefined}
        >
          <div className="kb-name">
            <b>
              {command.title}
              {command.plugin && <span className="kb-from">{command.category}</span>}
            </b>
            <code>{command.id}</code>
          </div>
          {command.shared && !isRecording && (
            <span className="kb-badge" title="Another command uses the same keys">
              <WarnIcon />
              Shared
            </span>
          )}
          <span className="kb-acts">
            {command.changed && (
              <button
                type="button"
                className="kb-button"
                data-quiet="true"
                onClick={() =>
                  change(
                    resetCommand(command, context),
                    `Reset ${command.title} to its default keys`,
                    command.id,
                  )
                }
              >
                Reset
              </button>
            )}
            <button
              type="button"
              className="kb-icon"
              aria-label={`Add a key for ${command.title}`}
              title="Add a key"
              onClick={() => record(command.id, null)}
            >
              <Glyph name="plus" size={13} />
            </button>
          </span>
          <div className="kb-binds">
            {command.keys.map((key) => {
              if (isRecording && recording.replace === key)
                return <span key={key.keys}>{recorder}</span>;
              if (key.off) {
                return (
                  <span
                    key={`off:${key.keys}`}
                    className="kb-bind"
                    data-off="true"
                    title="Default, unbound"
                  >
                    <Caps label={key.label} kind="off" />
                    <button
                      type="button"
                      className="kb-x"
                      aria-label={`Restore ${key.label}`}
                      title="Restore default"
                      onClick={() => change(restoreKey(key), null, command.id)}
                    >
                      <Glyph name="rotate-ccw" size={11} />
                    </button>
                  </span>
                );
              }
              const shared = key.shared.length > 0;
              const others = key.shared
                .map((id) => commands.find((other) => other.id === id)?.title ?? id)
                .join(", ");
              const title =
                (shared
                  ? `Also bound to ${others} · `
                  : key.source === "user"
                    ? "Set by you · "
                    : key.source === "workspace"
                      ? "Set by the workspace · "
                      : "") +
                (key.source === "workspace"
                  ? "Changed in the workspace's file"
                  : "Click to change");
              return (
                <span key={key.keys} className="kb-bind">
                  <button
                    type="button"
                    className="kb-edit"
                    title={title}
                    disabled={key.source === "workspace"}
                    onClick={() => (recording ? finish(null) : record(command.id, key))}
                  >
                    <Caps
                      label={key.label}
                      kind={shared ? "shared" : key.source === "user" ? "user" : undefined}
                    />
                  </button>
                  {key.source !== "workspace" && (
                    <button
                      type="button"
                      className="kb-x"
                      aria-label={`Unbind ${key.label}`}
                      title="Unbind"
                      onClick={() =>
                        change(
                          unbindKey(command, key, context),
                          `Unbound ${key.label} from ${command.title}`,
                          command.id,
                        )
                      }
                    >
                      <Glyph name="x" size={11} />
                    </button>
                  )}
                </span>
              );
            })}
            {isRecording && recording.replace === null && recorder}
            {command.keys.length === 0 && !isRecording && <span className="kb-none">—</span>}
          </div>
        </div>
        {isRecording && clashing.length === 0 && (
          <div className="kb-hint">
            <kbd>Enter</kbd> save · <kbd>Esc</kbd> cancel · <kbd>Backspace</kbd> clear · press a
            second combo for a chord
          </div>
        )}
        {isRecording && clashing.length > 0 && (
          <div className="kb-clash" role="alert">
            <WarnIcon />
            <span>
              <b>{recordedLabel}</b> already runs {clashing.map((other) => other.title).join(", ")}
            </span>
            <span className="kb-space" />
            <button
              type="button"
              className="kb-button"
              data-quiet="true"
              data-inherit="true"
              onClick={() => finish("keep")}
            >
              Keep both
            </button>
            <button type="button" className="kb-button" onClick={() => finish("move")}>
              Move it here
            </button>
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="commands-page kb">
      <p className="kb-blurb">
        Everything Lume can do, grouped by what it acts on, with the keys that run it now. Click a
        key to change it, or edit <code>[keybindings]</code> in config.toml.
      </p>
      <div className="kb-bar">
        <label className="kb-filter">
          <SearchIcon size={14} />
          <input
            ref={field}
            type="text"
            aria-label="Filter commands"
            placeholder={
              byKeys ? "Press the keys you’re looking for…" : "Filter by name, id or key"
            }
            autoComplete="off"
            spellCheck={false}
            readOnly={byKeys}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
          <button
            type="button"
            className="kb-icon"
            aria-pressed={byKeys}
            aria-label="Search by pressing keys"
            title="Search by pressing keys"
            onClick={() => {
              setByKeys(!byKeys);
              setQuery("");
              field.current?.focus();
            }}
          >
            <Glyph name="keyboard" size={15} />
          </button>
        </label>
      </div>
      <div className="kb-chips" role="group" aria-label="Show">
        {KEY_FILTERS.map(({ id, label }) => (
          <button
            key={id}
            type="button"
            className="kb-chip"
            aria-pressed={filter === id}
            onClick={() => setFilter(id)}
          >
            {label} <b>{commands.filter((command) => keptBy(id, command)).length}</b>
          </button>
        ))}
      </div>
      <div className="kb-legend">
        <Caps label={keysLabel("ctrl+t", context.platform)} />
        <span>Lume default</span>
        <Caps label={keysLabel("ctrl+t", context.platform)} kind="user" />
        <span>Set by you</span>
        <Caps label={keysLabel("ctrl+t", context.platform)} kind="shared" />
        <span>Shared with another command</span>
        <Caps label={keysLabel("ctrl+t", context.platform)} kind="off" />
        <span>Default you unbound</span>
      </div>
      {groups.map(({ group, commands: listed }) => (
        <section key={group} className="kb-group">
          <h2>
            {group}
            <span>{listed.length}</span>
          </h2>
          <div className="kb-card">{listed.map(row)}</div>
        </section>
      ))}
      {specs !== null && found.length === 0 && (
        <div className="kb-empty">
          {query.trim() === ""
            ? "No command is shown here."
            : `No command matches “${query.trim()}”.`}
        </div>
      )}
      {changed > 0 ? (
        <div className="kb-foot">
          <span>
            {changed} command{changed === 1 ? "" : "s"} changed · saved to config.toml
          </span>
          <button
            type="button"
            className="kb-button"
            data-quiet="true"
            onClick={() =>
              change(resetAll(context), "Every command is back on its default keys", null)
            }
          >
            Reset all
          </button>
        </div>
      ) : (
        <div className="kb-foot-space" />
      )}
      {toast && (
        <div className="kb-toast" role="status">
          <span>{toast.text}</span>
          {toast.undo && (
            <button
              type="button"
              className="kb-button"
              data-quiet="true"
              onClick={() => {
                const undo = toast.undo;
                setToast(null);
                if (undo) void apply(undo);
              }}
            >
              Undo
            </button>
          )}
        </div>
      )}
    </div>
  );
}
