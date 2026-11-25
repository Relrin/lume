/**
 * The command palette: every command the window has, found by typing a few letters
 * of its name.
 *
 * It opens over the window with the keyboard in its field. Typing narrows the list,
 * the arrow keys move through it, Enter runs what is marked and Escape puts the
 * palette away. A command that needs to be told something first, such as a name, has
 * the palette ask for it in the same field before it is run.
 *
 * Besides commands it finds the window's tabs, and what it calls hosts: the profiles a
 * terminal is opened from, the Rex servers and their sessions. Chips under the field choose what is searched, as do Tab
 * and Shift+Tab, or a sign at the start of what is typed: `>` commands, `@` tabs, `#`
 * hosts. Each chip counts what matches.
 *
 * Each row shows a picture for its kind, its title, a quieter line, and for a command
 * the keys that run it. With nothing typed the commands run lately come first, under
 * their own heading; with something typed the best match stands alone at the top.
 */

import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useStore } from "zustand";

import { keyLabel } from "../commands/store.ts";
import { KindIcon } from "../connect/icons.tsx";
import { kindOf, lineOf, openCommand } from "../connect/model.ts";
import type { CommandSpec, Profile } from "../core/types.ts";
import { Glyph } from "../icons/Glyph.tsx";
import type { LineGlyphName } from "../icons/glyphs.generated.ts";
import { listed, moreLine, type Listed } from "../listed.ts";
import { lookup } from "../lookup.ts";
import { LinkDot } from "../rex/LinkDot.tsx";
import { useRexLook } from "../rex/look.ts";
import type { RexLink } from "../rex/model.ts";
import { useSlotMotion } from "../motion/slots.ts";
import { selectTabOrder, selectTitlePane } from "../layout/selectors.ts";
import type { ShellApp } from "../shell/boot.ts";
import { useShell } from "../shell/context.ts";
import { describeNow, describeTabNow } from "../shell/describing.ts";
import { windowWork, type WorkState } from "../shell/work.ts";
import { SearchIcon } from "../shell/icons.tsx";
import { PALETTE_RESULTS_MAX } from "../tuning.ts";
import { promptsFor, readArg, type ArgPrompt, type Choice } from "./args.ts";
import { profileChoices, workspaceChoices } from "./choices.ts";
import { rank } from "./fuzzy.ts";
import { Cap, KeyCaps } from "./KeyCaps.tsx";
import { rexTargets, type RexTarget } from "./rex.ts";
import { PER_KIND_MAX, readSign, SCOPES, stepScope, type Scope } from "./scopes.ts";
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
  /** The heading of the part of the list the row is in, or none. */
  readonly section: string;
  /** How well the query matched, to find the best of every kind. */
  readonly score: number;
  /** The picture before a tab's or a host's name. */
  readonly icon?: ReactNode;
  /** The quieter line after a tab's or a host's name. */
  readonly sub?: string;
  /** How the work in a tab stands, as a coloured dot and a word. */
  readonly state?: WorkState;
  /** Whether a Rex server can be used, as a coloured dot. */
  readonly link?: RexLink;
  /** A word at the row's end. */
  readonly meta?: string;
  /** Whether it is what the window shows: it is listed however much there is before it. */
  readonly on?: boolean;
  /** What choosing a tab or a host does. */
  readonly run?: () => void;
}

/** A tab as the palette finds it. */
interface TabEntry {
  readonly tab: string;
  readonly label: string;
  readonly where: string;
  readonly state: WorkState | null;
}

/** How each state of a tab's work is said at the end of its row. */
const STATE_WORDS: Readonly<Record<WorkState, string>> = {
  wait: "needs you",
  work: "working",
  done: "done",
  failed: "failed",
};

/** How much each state presses, to show the one that presses most of a tab's panes. */
const STATE_ORDER: Readonly<Record<WorkState, number>> = { wait: 0, failed: 1, done: 2, work: 3 };

/** The window's tabs, in their order, as the palette lists them. */
function tabEntries(app: ShellApp): TabEntry[] {
  const state = app.layout.getState();
  if (!state.ready) return [];
  const work = windowWork(app);
  return selectTabOrder(state).map((tab) => {
    const pane = selectTitlePane(state, tab);
    const pressing = work
      .filter((item) => item.tab === tab)
      .sort((a, b) => STATE_ORDER[a.state] - STATE_ORDER[b.state])[0];
    return {
      tab,
      label: describeTabNow(app, tab).label,
      where: pane === undefined ? "" : describeNow(app, pane).where,
      state: pressing?.state ?? null,
    };
  });
}

/** The picture of a command's kind, by its category. */
const CATEGORY_GLYPHS: Readonly<Record<string, LineGlyphName>> = {
  Tab: "panel-top",
  Pane: "columns-2",
  Application: "settings",
  Terminal: "terminal",
  Workspace: "layers",
  View: "monitor",
  Window: "square",
  Panel: "panel-right",
  Search: "search",
  Session: "activity",
  Host: "server",
  Configuration: "settings",
  Plugins: "puzzle",
  Git: "git-branch",
};

function categoryGlyph(category: string): LineGlyphName {
  return lookup(CATEGORY_GLYPHS, category) ?? "chevron-right";
}

/** The picture of each scope's chip. */
const SCOPE_GLYPHS: Readonly<Record<Exclude<Scope, "all">, LineGlyphName>> = {
  commands: "terminal",
  tabs: "panel-top",
  hosts: "server",
};

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

const NOTHING: Listed<Row> = { rows: [], more: 0 };

/** What the palette searches, as the dialog holds it. */
interface Sources {
  readonly commands: readonly CommandSpec[];
  readonly recents: readonly string[];
  readonly keysOf: (id: string) => string | undefined;
  readonly tabs: readonly TabEntry[];
  readonly hosts: readonly Profile[];
  readonly defaultHost: string | null;
  readonly showTab: (tab: string) => void;
  readonly openHost: (profile: string) => void;
  readonly rex: readonly RexTarget[];
  readonly openRex: (target: RexTarget) => void;
}

/** The commands that match, as rows, best first. */
function commandMatches(sources: Sources, query: string): Row[] {
  const offered = sources.commands.filter(
    (command) => command.palette && command.id !== OWN_COMMAND,
  );
  return rank(offered, query, (command) => `${command.category}: ${command.title}`).map(
    (match) => ({
      key: match.item.id,
      prefix: `${match.item.category}: `,
      text: match.item.title,
      indices: match.indices,
      keys: sources.keysOf(match.item.id),
      command: match.item,
      section: "Commands",
      score: match.score,
    }),
  );
}

/** The tabs that match, as rows, best first. */
function tabMatches(sources: Sources, query: string): Row[] {
  return rank(sources.tabs, query, (entry) => entry.label).map((match) => ({
    key: `tab:${match.item.tab}`,
    prefix: "",
    text: match.item.label,
    indices: match.indices,
    keys: undefined,
    command: undefined,
    section: "Tabs",
    score: match.score,
    icon: <Glyph name="panel-top" size={15} weight={1.9} />,
    sub: match.item.where,
    ...(match.item.state === null
      ? {}
      : { state: match.item.state, meta: STATE_WORDS[match.item.state] }),
    run: () => sources.showTab(match.item.tab),
  }));
}

/** The Rex servers and sessions that match, as rows, best first. */
function rexMatches(sources: Sources, query: string): Row[] {
  return rank(sources.rex, query, (target) => target.text).map((match) => ({
    key: match.item.key,
    prefix: "",
    text: match.item.text,
    indices: match.indices,
    keys: undefined,
    command: undefined,
    section: "Hosts",
    score: match.score,
    icon: (
      <span className="palette-host-icon" data-kind="rex">
        <Glyph name={match.item.kind === "host" ? "server" : "terminal"} size={15} />
      </span>
    ),
    sub: match.item.sub,
    link: match.item.link,
    on: match.item.on,
    ...(match.item.meta === "" ? {} : { meta: match.item.meta }),
    run: () => sources.openRex(match.item),
  }));
}

/** The profiles that match, as rows, best first. */
function profileMatches(sources: Sources, query: string): Row[] {
  return rank(sources.hosts, query, (profile) => profile.name).map((match) => ({
    key: `host:${match.item.id}`,
    prefix: "",
    text: match.item.name,
    indices: match.indices,
    keys: undefined,
    command: undefined,
    section: "Hosts",
    score: match.score,
    icon: (
      <span className="palette-host-icon" data-kind={kindOf(match.item)}>
        <KindIcon kind={kindOf(match.item)} size={15} />
      </span>
    ),
    sub: lineOf(match.item),
    ...(match.item.id === sources.defaultHost ? { meta: "default" } : {}),
    run: () => sources.openHost(match.item.id),
  }));
}

/**
 * The hosts that match: the Rex servers with their sessions, then the profiles, and with
 * something typed the best of both first.
 */
function hostMatches(sources: Sources, query: string): Row[] {
  const rows = [...rexMatches(sources, query), ...profileMatches(sources, query)];
  return query.trim() === "" ? rows : rows.sort((a, b) => b.score - a.score);
}

/** The rows of a list that is shown whole, as far as it goes. */
function firstRows(rows: readonly Row[]): Listed<Row> {
  return listed(rows, PALETTE_RESULTS_MAX, (row) => row.on === true);
}

/** Rows under another heading. */
function under(rows: readonly Row[], section: string): Row[] {
  return rows.map((row) => ({ ...row, section }));
}

/**
 * The rows of a scope. Commands run lately come first while nothing is typed; under
 * everything with something typed, the best match of any kind stands alone at the top
 * and each kind lists a few of its best after it. How many there are besides is said of
 * a scope of its own: of everything, the chips say it.
 */
function scopedRows(sources: Sources, scope: Scope, query: string): Listed<Row> {
  const typed = query.trim() !== "";
  const commands = commandMatches(sources, query);
  const recent = (): Row[] => {
    const lately = sources.recents.flatMap((id) => commands.filter((row) => row.key === id));
    return [
      ...under(lately, "Recent"),
      ...commands.filter((row) => !sources.recents.includes(row.key)),
    ];
  };
  if (scope === "commands") return firstRows(typed ? commands : recent());
  if (scope === "tabs") return firstRows(tabMatches(sources, query));
  if (scope === "hosts") return firstRows(hostMatches(sources, query));
  const tabs = tabMatches(sources, query);
  const hosts = hostMatches(sources, query);
  if (!typed) {
    const lately = recent();
    const firstCommands = lately.filter((row) => row.section === "Recent");
    const otherCommands = lately.filter((row) => row.section !== "Recent");
    const rows = [
      ...firstCommands,
      ...tabs.slice(0, PER_KIND_MAX),
      ...hosts.slice(0, PER_KIND_MAX),
      ...otherCommands,
    ];
    return { rows: rows.slice(0, PALETTE_RESULTS_MAX), more: 0 };
  }
  // The sort is stable: of equal matches, commands stay first.
  const best = [...commands, ...tabs, ...hosts].sort((a, b) => b.score - a.score)[0];
  if (!best) return NOTHING;
  const rest = (rows: readonly Row[]) => rows.filter((row) => row !== best).slice(0, PER_KIND_MAX);
  const rows = [...rest(commands), ...rest(tabs), ...rest(hosts)];
  return { rows: [{ ...best, section: "Top hit" }, ...rows], more: 0 };
}

/** How many of each scope match what is typed, for the chips. */
function scopeCounts(sources: Sources, query: string): Readonly<Record<Scope, number>> {
  const commands = commandMatches(sources, query).length;
  const tabs = rank(sources.tabs, query, (entry) => entry.label).length;
  const hosts =
    rank(sources.hosts, query, (profile) => profile.name).length +
    rank(sources.rex, query, (target) => target.text).length;
  return { all: commands + tabs + hosts, commands, tabs, hosts };
}

function choiceRows(choices: readonly Choice[], query: string): Listed<Row> {
  return firstRows(
    rank(choices, query, (choice) => choice.label).map((match) => ({
      key: match.item.value,
      prefix: "",
      text: match.item.label,
      indices: match.indices,
      keys: undefined,
      command: undefined,
      value: match.item.value,
      section: "",
      score: match.score,
    })),
  );
}

function PaletteDialog({ first }: { first: CommandSpec | undefined }) {
  const app = useShell();
  const commands = useStore(app.commandStore, (state) => state.commands);
  const view = useStore(app.config, (state) => state.view);
  const profiles = useStore(app.profiles, (state) => state.list);
  const servers = useStore(app.rex, (state) => state.list);
  const backend = useStore(app.layout, (state) => state.backend);
  useRexLook();
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
  const [scope, setScope] = useState<Scope>("all");
  // The marked row, by its key: servers come and go while the palette is open, and the
  // mark stays on what it was on. Nothing means the first row.
  const [selected, setSelected] = useState<string | null>(null);
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

  // The tabs are read once, as the palette opens: choosing one puts the palette away.
  const tabs = useMemo(() => tabEntries(app), [app]);
  const sources = useMemo((): Sources => {
    const keysOf = (id: string) =>
      view ? keyLabel(view.keymap.bindings, id, view.platform) : undefined;
    return {
      commands,
      recents,
      keysOf,
      tabs,
      hosts: profiles?.profiles ?? [],
      defaultHost: profiles?.default ?? null,
      showTab: (tab) => {
        app.client
          .layoutApply({ type: "tab_activate", target: { type: "id", tab } })
          .catch(() => undefined);
      },
      openHost: (profile) => {
        const command = openCommand(profile, "tab", app.views.focused() !== undefined);
        app.commands.run(command.id, command.args);
      },
      rex: rexTargets(servers, backend),
      openRex: (target) => app.commands.run(target.command, target.args),
    };
  }, [app, commands, recents, view, tabs, profiles, servers, backend]);
  const { rows, more } = useMemo(() => {
    if (stage.kind === "commands") return scopedRows(sources, scope, query);
    return choices ? choiceRows(choices, query) : NOTHING;
  }, [stage.kind, sources, scope, query, choices]);
  const counts = useMemo(() => scopeCounts(sources, query), [sources, query]);
  const pickScope = (next: Scope) => {
    setScope(next);
    setSelected(null);
  };
  const marked = Math.max(
    0,
    rows.findIndex((row) => row.key === selected),
  );
  const firstKey = rows[0]?.key;
  useEffect(() => {
    // Once the list is known its first row is the one marked, whatever comes before it.
    if (selected === null && firstKey !== undefined && servers !== null) setSelected(firstKey);
  }, [selected, firstKey, servers]);

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
    setSelected(null);
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
    setSelected(null);
    setProblem(null);
  };

  const accept = (index: number) => {
    const row = rows[index];
    if (stage.kind === "commands") {
      if (row?.run) {
        close();
        row.run();
        return;
      }
      if (row?.command) choose(row.command);
      return;
    }
    // A row of the list is taken for what it stands for; anything else as it was typed.
    answer(row ? (row.value ?? row.text) : query);
  };

  const move = (to: number) => {
    const row = rows[((to % rows.length) + rows.length) % rows.length];
    if (row) setSelected(row.key);
  };

  const placeholder =
    stage.kind === "commands" ? "Search commands, tabs and hosts" : (prompt?.placeholder ?? "");
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
              const typed = event.target.value;
              // A sign typed first chooses what is searched, while everything is.
              const sign = stage.kind === "commands" && scope === "all" ? readSign(typed) : null;
              if (sign) setScope(sign.scope);
              setQuery(sign ? sign.rest : typed);
              setSelected(null);
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
                case "Tab":
                  if (stage.kind !== "commands") return;
                  pickScope(stepScope(scope, event.shiftKey ? -1 : 1));
                  break;
                case "Backspace":
                  // An empty field goes back to searching everything.
                  if (stage.kind !== "commands" || query !== "" || scope === "all") {
                    event.stopPropagation();
                    return;
                  }
                  pickScope("all");
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
        {stage.kind === "commands" && (
          <div className="palette-chips" role="group" aria-label="Search">
            {SCOPES.map((choice) => (
              <button
                key={choice.value}
                type="button"
                className="palette-chip"
                aria-label={`${choice.label}, ${counts[choice.value]}`}
                aria-pressed={scope === choice.value}
                tabIndex={-1}
                onPointerDown={(event) => event.preventDefault()}
                onClick={() => pickScope(choice.value)}
              >
                {choice.value !== "all" && (
                  <Glyph name={SCOPE_GLYPHS[choice.value]} size={12} weight={2} />
                )}
                {choice.label}
                <span className="palette-chip-count" aria-hidden="true">
                  {counts[choice.value]}
                </span>
              </button>
            ))}
          </div>
        )}
        {problem !== null && (
          <p className="palette-problem" role="alert">
            {problem}
          </p>
        )}
        <div ref={list} id={LIST_ID} className="palette-list" role="listbox" aria-label={label}>
          {rows.map((row, index) => {
            const heading = row.section !== "" && row.section !== rows[index - 1]?.section;
            return (
              <Fragment key={row.key}>
                {heading && (
                  <div className="palette-heading" role="presentation">
                    {row.section}
                  </div>
                )}
                <div
                  id={optionId(index)}
                  className="palette-row"
                  role="option"
                  aria-selected={index === marked}
                  onPointerMove={() => {
                    if (index !== marked) setSelected(row.key);
                  }}
                  onClick={() => accept(index)}
                >
                  {row.command ? (
                    <span className="palette-row-icon">
                      <Glyph name={categoryGlyph(row.command.category)} size={15} weight={1.9} />
                    </span>
                  ) : row.icon ? (
                    <span className="palette-row-icon">{row.icon}</span>
                  ) : null}
                  <span className="palette-row-text">
                    <Marked text={row.text} indices={row.indices} offset={[...row.prefix].length} />
                  </span>
                  {row.link !== undefined && (
                    <LinkDot link={row.link} className="palette-row-link" />
                  )}
                  {row.state !== undefined && (
                    <i className="palette-row-state" data-state={row.state} />
                  )}
                  <span className="palette-row-sub">
                    {row.command ? (
                      <Marked text={row.command.category} indices={row.indices} offset={0} />
                    ) : (
                      row.sub
                    )}
                  </span>
                  {row.keys !== undefined && (
                    <KeyCaps label={row.keys} className="palette-row-keys" />
                  )}
                  {row.meta !== undefined && <span className="palette-row-meta">{row.meta}</span>}
                </div>
              </Fragment>
            );
          })}
          {more > 0 && (
            <div className="palette-more" role="presentation">
              {moreLine(more)}
            </div>
          )}
        </div>
        {rows.length === 0 && (
          <p className="palette-empty">
            {stage.kind === "commands"
              ? "Nothing matches."
              : choices === null && source !== null
                ? "Loading…"
                : "Press Enter to confirm, Escape to cancel."}
          </p>
        )}
        <div className="palette-foot">
          <span>
            <Cap>↵</Cap>
            {stage.kind === "commands" ? "Run" : "Confirm"}
          </span>
          <span>
            <Cap>↑</Cap>
            <Cap>↓</Cap>
            Move
          </span>
          {stage.kind === "commands" && (
            <span>
              <Cap>Tab</Cap>
              Filter
            </span>
          )}
          {stage.kind === "commands" && (
            <span>
              <Cap>&gt;</Cap>
              <Cap>@</Cap>
              <Cap>#</Cap>
              Commands, tabs, hosts
            </span>
          )}
          <span>
            <Cap>Esc</Cap>
            Close
          </span>
          {stage.kind === "args" && stage.prompts.length > 1 && (
            <span className="palette-foot-end">
              {stage.at + 1} of {stage.prompts.length}
            </span>
          )}
        </div>
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
