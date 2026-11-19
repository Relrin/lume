/**
 * A choice a plugin asks the user to make from a list, drawn like the command palette.
 *
 * The plugin's name heads the field, so the user knows whose list it is. Typing narrows
 * the list, the arrow keys move through it, Enter chooses what is marked and Escape
 * puts it away, which chooses nothing. The plugin's words are shown as text.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import { rank } from "../palette/fuzzy.ts";
import { SearchIcon } from "../shell/icons.tsx";
import type { PickItem, Prompt } from "../terminal/prompts.ts";
import { PALETTE_RESULTS_MAX } from "../tuning.ts";

const LIST_ID = "plugin-pick-list";

function optionId(index: number): string {
  return `plugin-pick-option-${index}`;
}

/** The list a pick shows for what is typed: all of it at first, then what matches. */
export function pickRows(items: readonly PickItem[], query: string): PickItem[] {
  if (query.trim() === "") return items.slice(0, PALETTE_RESULTS_MAX);
  return rank(items, query, (item) => item.label)
    .slice(0, PALETTE_RESULTS_MAX)
    .map((match) => match.item);
}

export function PluginPickDialog({ prompt }: { prompt: Extract<Prompt, { kind: "pick" }> }) {
  const { ask } = prompt;
  const field = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const rows = useMemo(() => pickRows(ask.items, query), [ask.items, query]);
  const marked = Math.min(selected, Math.max(0, rows.length - 1));
  const label = ask.title ?? `Choose for ${ask.plugin.name}`;

  useEffect(() => {
    field.current?.focus();
  }, []);

  useEffect(() => {
    list.current?.querySelector(`#${optionId(marked)}`)?.scrollIntoView({ block: "nearest" });
  }, [marked, rows]);

  const choose = (index: number) => {
    const row = rows[index];
    if (row) prompt.answer(true, row.id);
  };
  const move = (to: number) => {
    if (rows.length === 0) return;
    setSelected(((to % rows.length) + rows.length) % rows.length);
  };
  const expanded = rows.length > 0;

  return (
    <div className="palette-backdrop" onPointerDown={() => prompt.answer(false)}>
      <div
        className="palette plugin-pick"
        role="dialog"
        aria-modal="true"
        aria-label={label}
        onPointerDown={(event) => event.stopPropagation()}
      >
        <div className="palette-field">
          <span className="palette-field-icon">
            <SearchIcon />
          </span>
          <span className="palette-crumb">{ask.plugin.name}</span>
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
            placeholder={ask.placeholder ?? ask.title ?? "Type to narrow the list"}
            spellCheck={false}
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setSelected(0);
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
                  choose(marked);
                  break;
                case "Escape":
                  prompt.answer(false);
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
        {ask.title ? <p className="plugin-pick-title">{ask.title}</p> : null}
        <div ref={list} id={LIST_ID} className="palette-list" role="listbox" aria-label={label}>
          {rows.map((row, index) => (
            <div
              key={row.id}
              id={optionId(index)}
              className="palette-row"
              role="option"
              aria-selected={index === marked}
              onPointerMove={() => {
                if (index !== marked) setSelected(index);
              }}
              onClick={() => choose(index)}
            >
              <span className="palette-row-text">{row.label}</span>
              {row.detail ? <span className="plugin-pick-detail">{row.detail}</span> : null}
            </div>
          ))}
        </div>
        {rows.length === 0 && <p className="palette-empty">Nothing matches.</p>}
      </div>
    </div>
  );
}
