/**
 * A pane's search bar: a field over the top of its terminal, with how many matches
 * there are and the way to the next one.
 *
 * The bar renders when it is shown and when it is put away, and at no other time. What
 * it says while a search runs is written to its elements directly: a search of a long
 * scrollback reports many times a second, and so does a terminal that prints while it
 * is being searched.
 */

import { useLayoutEffect, useRef } from "react";
import { useStore } from "zustand";

import { selectFocus } from "../layout/selectors.ts";
import type { PaneId } from "../layout/types.ts";
import { useShell } from "../shell/context.ts";
import { useKeyHint } from "../shell/hints.ts";
import { searchLabel } from "./controller.ts";

function Arrow({ up }: { up: boolean }) {
  return (
    <svg width="12" height="12" viewBox="0 0 24 24" aria-hidden="true">
      <polyline
        points={up ? "6 15 12 9 18 15" : "6 9 12 15 18 9"}
        fill="none"
        stroke="currentColor"
        strokeWidth="2.2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function Cross() {
  return (
    <svg width="12" height="12" viewBox="0 0 24 24" aria-hidden="true">
      <path
        d="M6 6l12 12M18 6L6 18"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.2"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** The search bar of a pane, while the pane is being searched. */
export function SearchBar({ pane }: { pane: PaneId }) {
  const app = useShell();
  const open = useStore(app.search.store, (state) => state.panes[pane]?.open ?? false);
  return open ? <OpenSearchBar pane={pane} /> : null;
}

function OpenSearchBar({ pane }: { pane: PaneId }) {
  const app = useShell();
  const bar = useRef<HTMLDivElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const count = useRef<HTMLSpanElement>(null);
  const older = useRef<HTMLButtonElement>(null);
  const newer = useRef<HTMLButtonElement>(null);
  const closeHint = useKeyHint("lume.search.close");

  useLayoutEffect(() => {
    const input = field.current;
    if (!input) return undefined;
    const focus = () => {
      input.focus({ preventScroll: true });
      input.select();
    };
    const forget = app.search.registerBar(pane, { focus });
    const write = () => {
      const search = app.search.of(pane);
      // What is looked for can be set from outside the bar, by a command.
      if (input.value !== search.query) input.value = search.query;
      if (count.current) count.current.textContent = searchLabel(search);
      const none = search.total === 0;
      if (bar.current) {
        bar.current.dataset.none = String(none && !search.running && search.query !== "");
      }
      if (older.current) older.current.disabled = none;
      if (newer.current) newer.current.disabled = none;
    };
    write();
    const stop = app.search.store.subscribe(write);
    // A bar that comes back with its tab does not take the keyboard from the terminal.
    if (app.search.of(pane).keyboard === "bar" && selectFocus(app.layout.getState()) === pane) {
      focus();
    }
    return () => {
      stop();
      forget();
    };
  }, [app, pane]);

  const close = () => {
    app.search.close(pane);
    app.views.get(pane)?.focus();
  };
  // The keyboard stays in the field when a button of the bar is pressed.
  const keep = (event: React.PointerEvent) => event.preventDefault();

  return (
    <div className="search-anchor">
      <div
        className="search-bar"
        ref={bar}
        role="search"
        aria-label="Search this pane"
        data-search-bar
      >
        <input
          ref={field}
          className="search-field"
          type="text"
          aria-label="Search for"
          placeholder="Search"
          autoCapitalize="off"
          autoComplete="off"
          autoCorrect="off"
          spellCheck={false}
          onInput={(event) => app.search.setQuery(pane, event.currentTarget.value)}
          onFocus={() => {
            app.search.setKeyboard(pane, "bar");
            // The pane whose bar is typed in is the pane the user is working in.
            if (selectFocus(app.layout.getState()) !== pane) {
              app.client
                .layoutApply({ type: "focus", target: { type: "pane", pane } })
                .catch(() => undefined);
            }
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              app.search.step(pane, event.shiftKey ? "newer" : "older");
            } else if (event.key === "Escape") {
              event.preventDefault();
              close();
            }
          }}
        />
        <span className="search-count" ref={count} aria-live="polite" />
        <button
          ref={older}
          type="button"
          className="control search-button search-step"
          aria-label="Previous match"
          title="Previous match (Enter)"
          onPointerDown={keep}
          onClick={() => app.search.step(pane, "older")}
        >
          <Arrow up />
        </button>
        <button
          ref={newer}
          type="button"
          className="control search-button search-step"
          aria-label="Next match"
          title="Next match (Shift+Enter)"
          onPointerDown={keep}
          onClick={() => app.search.step(pane, "newer")}
        >
          <Arrow up={false} />
        </button>
        <button
          type="button"
          className="control search-button"
          aria-label="Close search"
          title={`Close search${closeHint || " (Esc)"}`}
          onPointerDown={keep}
          onClick={close}
        >
          <Cross />
        </button>
      </div>
    </div>
  );
}
