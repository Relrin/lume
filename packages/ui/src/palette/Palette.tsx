/**
 * The command palette: every command the window has, found by typing a few letters
 * of its name.
 *
 * It opens over the window with the keyboard in its field. Typing narrows the list,
 * the arrow keys move through it, Enter runs what is marked and Escape puts the
 * palette away. A command that needs to be told something first, such as a name, has
 * the palette ask for it in the same field before it is run.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "zustand";

import { keyLabel } from "../commands/store.ts";
import type { CommandSpec } from "../core/types.ts";
import { useSlotMotion } from "../motion/slots.ts";
import { useShell } from "../shell/context.ts";
import { SearchIcon } from "../shell/icons.tsx";
import { PALETTE_RESULTS_MAX } from "../tuning.ts";
import { promptsFor, readArg, type ArgPrompt, type Choice } from "./args.ts";
import { profileChoices, workspaceChoices } from "./choices.ts";
import { rank } from "./fuzzy.ts";
import { pageStorage, readRecents, rememberRecent } from "./store.ts";

/** What the palette is asking for. */
type Stage =
  | { readonly kind: "commands" }
  | {
      readonly kind: "args";
      readonly command: CommandSpec;
      readonly prompts: readonly ArgPrompt[];
      /** Which of the prompts is being asked. */
      readonly at: number;
      /** What was answered so far, by argument. */
      readonly values: Readonly<Record<string, string | number>>;
    };

/** One row of the list. */
interface Row {
  readonly key: string;
  /** What is shown before the row's text, in a quieter colour. */
  readonly prefix: string;
  readonly text: string;
  /** Which characters of the prefix and the text the query matched, counted across both. */
  readonly indices: readonly number[];
  /** The keys that run the row's command. */
  readonly keys: string | undefined;
  readonly command: CommandSpec | undefined;
  /** What a row chosen from a list stands for, when that is not the text it shows. */
  readonly value?: string;
}

/** The command that shows the palette, which the palette has no reason to list. */
const OWN_COMMAND = "lume.palette.open";

const LIST_ID = "palette-list";

function optionId(index: number): string {
  return `palette-option-${index}`;
}

/** A piece of a row's text with the characters the query matched marked. */
function Marked({
  text,
  indices,
  offset,
}: {
  text: string;
  indices: readonly number[];
  /** Where in the row this piece begins, in characters. */
  offset: number;
}) {
  const marked = new Set(indices);
  // Runs of characters that are alike in being marked or not, each as one element.
  const runs: { readonly text: string; readonly marked: boolean }[] = [];
  [...text].forEach((character, index) => {
    const isMarked = marked.has(offset + index);
    const last = runs[runs.length - 1];
    if (last && last.marked === isMarked) {
      runs[runs.length - 1] = { text: last.text + character, marked: isMarked };
    } else {
      runs.push({ text: character, marked: isMarked });
    }
  });
  return (
    <>
      {runs.map((run, index) =>
        run.marked ? (
          <mark key={index} className="palette-mark">
            {run.text}
          </mark>
        ) : (
          <span key={index}>{run.text}</span>
        ),
      )}
    </>
  );
}

/** The commands the palette offers, as rows: those run lately first while nothing is typed. */
function commandRows(
  commands: readonly CommandSpec[],
  query: string,
  recents: readonly string[],
  keysOf: (id: string) => string | undefined,
): Row[] {
  const offered = commands.filter((command) => command.palette && command.id !== OWN_COMMAND);
  const row = (command: CommandSpec, indices: readonly number[]): Row => ({
    key: command.id,
    prefix: `${command.category}: `,
    text: command.title,
    indices,
    keys: keysOf(command.id),
    command,
  });
  if (query.trim() === "") {
    const lately = recents.flatMap((id) => offered.filter((command) => command.id === id));
    const others = offered.filter((command) => !recents.includes(command.id));
    return [...lately, ...others].slice(0, PALETTE_RESULTS_MAX).map((command) => row(command, []));
  }
  return rank(offered, query, (command) => `${command.category}: ${command.title}`)
    .slice(0, PALETTE_RESULTS_MAX)
    .map((match) => row(match.item, match.indices));
}

function choiceRows(choices: readonly Choice[], query: string): Row[] {
  return rank(choices, query, (choice) => choice.label)
    .slice(0, PALETTE_RESULTS_MAX)
    .map((match) => ({
      key: match.item.value,
      prefix: "",
      text: match.item.label,
      indices: match.indices,
      keys: undefined,
      command: undefined,
      value: match.item.value,
    }));
}

function PaletteDialog({ first }: { first: CommandSpec | undefined }) {
  const app = useShell();
  const commands = useStore(app.commandStore, (state) => state.commands);
  const view = useStore(app.config, (state) => state.view);
  const profiles = useStore(app.profiles, (state) => state.list);
  const field = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLDivElement>(null);
  useSlotMotion("palette.enter", box);
  const storage = useMemo(pageStorage, []);
  const recents = useMemo(() => readRecents(storage), [storage]);

  const [stage, setStage] = useState<Stage>(() =>
    first
      ? { kind: "args", command: first, prompts: promptsFor(first), at: 0, values: {} }
      : { kind: "commands" },
  );
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [problem, setProblem] = useState<string | null>(null);
  /** The workspaces a prompt's value is chosen from, once they have been fetched. */
  const [workspaces, setWorkspaces] = useState<readonly Choice[] | null>(null);

  const prompt = stage.kind === "args" ? stage.prompts[stage.at] : undefined;
  const source = prompt?.choices ?? null;
  // The profiles are kept up to date for the whole window; the workspaces are asked
  // for each time they are needed, since other windows make and remove them.
  const choices = useMemo(() => {
    if (source === "profiles") return profiles ? profileChoices(profiles) : null;
    if (source === "workspaces") return workspaces;
    return null;
  }, [source, profiles, workspaces]);

  // Putting the palette away gives the keyboard back to where the user was working.
  const close = () => app.palette.getState().close();
  const run = (command: CommandSpec, values: Readonly<Record<string, string | number>>) => {
    rememberRecent(storage, command.id);
    close();
    app.commands.run(command.id, Object.keys(values).length === 0 ? null : values);
  };

  useEffect(() => {
    field.current?.focus();
  }, [stage]);

  // A command that was opened for and asks nothing is simply run, once.
  const ran = useRef(false);
  useEffect(() => {
    if (stage.kind !== "args" || stage.prompts.length > 0 || ran.current) return;
    ran.current = true;
    run(stage.command, {});
  });

  useEffect(() => {
    setWorkspaces(null);
    if (source !== "workspaces") return undefined;
    let current = true;
    app.client
      .workspacesList()
      .then(workspaceChoices)
      .then(
        (found) => {
          if (current) setWorkspaces(found);
        },
        () => {
          // Without the list the name can still be typed.
          if (current) setWorkspaces([]);
        },
      );
    return () => {
      current = false;
    };
  }, [app, source]);

  const rows = useMemo(() => {
    if (stage.kind === "commands") {
      const keysOf = (id: string) =>
        view ? keyLabel(view.keymap.bindings, id, view.platform) : undefined;
      return commandRows(commands, query, recents, keysOf);
    }
    return choices ? choiceRows(choices, query) : [];
  }, [stage.kind, commands, query, recents, view, choices]);
  const marked = Math.min(selected, Math.max(0, rows.length - 1));

  useEffect(() => {
    list.current?.querySelector(`#${optionId(marked)}`)?.scrollIntoView({ block: "nearest" });
  }, [marked, rows]);

  const choose = (command: CommandSpec) => {
    const prompts = promptsFor(command);
    if (prompts.length === 0) {
      run(command, {});
      return;
    }
    setStage({ kind: "args", command, prompts, at: 0, values: {} });
    setQuery("");
    setSelected(0);
    setProblem(null);
  };

  const answer = (typed: string) => {
    if (stage.kind !== "args" || !prompt) return;
    const read = readArg(prompt, typed);
    if (!read.ok) {
      setProblem(read.problem);
      return;
    }
    const values = { ...stage.values, [prompt.name]: read.value };
    if (stage.at + 1 >= stage.prompts.length) {
      run(stage.command, values);
      return;
    }
    setStage({ ...stage, at: stage.at + 1, values });
    setQuery("");
    setSelected(0);
    setProblem(null);
  };

  const accept = (index: number) => {
    const row = rows[index];
    if (stage.kind === "commands") {
      if (row?.command) choose(row.command);
      return;
    }
    // A row of the list is taken for what it stands for; anything else as it was typed.
    answer(row ? (row.value ?? row.text) : query);
  };

  const move = (to: number) => {
    if (rows.length === 0) return;
    setSelected(((to % rows.length) + rows.length) % rows.length);
  };

  const placeholder = stage.kind === "commands" ? "Type a command" : (prompt?.placeholder ?? "");
  const label = stage.kind === "commands" ? "Command" : (prompt?.placeholder ?? "Value");
  const expanded = rows.length > 0;

  return (
    <div className="palette-backdrop" onPointerDown={close}>
      <div
        ref={box}
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <div className="palette-field">
          <span className="palette-field-icon">
            <SearchIcon />
          </span>
          {stage.kind === "args" && <span className="palette-crumb">{stage.command.title}</span>}
          <input
            ref={field}
            className="palette-input"
            type="text"
            role="combobox"
            aria-label={label}
            aria-expanded={expanded}
            aria-controls={LIST_ID}
            aria-autocomplete="list"
            aria-activedescendant={expanded ? optionId(marked) : undefined}
            aria-invalid={problem !== null}
            placeholder={placeholder}
            spellCheck={false}
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setSelected(0);
              setProblem(null);
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
                case "Enter":
                  accept(marked);
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
        </div>
        {problem !== null && (
          <p className="palette-problem" role="alert">
            {problem}
          </p>
        )}
        <div ref={list} id={LIST_ID} className="palette-list" role="listbox" aria-label={label}>
          {rows.map((row, index) => (
            <div
              key={row.key}
              id={optionId(index)}
              className="palette-row"
              role="option"
              aria-selected={index === marked}
              onPointerMove={() => {
                if (index !== marked) setSelected(index);
              }}
              onClick={() => accept(index)}
            >
              <span className="palette-row-text">
                {row.prefix !== "" && (
                  <span className="palette-row-prefix">
                    <Marked text={row.prefix} indices={row.indices} offset={0} />
                  </span>
                )}
                <Marked text={row.text} indices={row.indices} offset={[...row.prefix].length} />
              </span>
              {row.keys !== undefined && <span className="palette-row-keys">{row.keys}</span>}
            </div>
          ))}
        </div>
        {rows.length === 0 && (
          <p className="palette-empty">
            {stage.kind === "commands"
              ? "No command matches."
              : choices === null && source !== null
                ? "Loading…"
                : "Press Enter to confirm, Escape to cancel."}
          </p>
        )}
      </div>
    </div>
  );
}

/** The palette, while it is on show. */
export function Palette() {
  const app = useShell();
  const shown = useStore(app.palette, (state) => state.shown);
  const opening = useStore(app.palette, (state) => state.opening);
  const command = useStore(app.palette, (state) => state.command);
  const first = useStore(app.commandStore, (state) =>
    command === null ? undefined : state.byId.get(command),
  );
  if (!shown) return null;
  // Each opening is a new dialog: nothing of the last one's query or stage is left.
  return <PaletteDialog key={opening} first={first} />;
}
