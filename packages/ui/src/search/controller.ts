/**
 * Searching the window's terminals: what each pane's search bar shows, and when the
 * searching is done.
 *
 * A terminal's view does the looking, a few milliseconds at a time when it is asked to.
 * This is what asks. A long scrollback takes many such stretches, and between any two
 * the page draws a frame or takes a key, so a search never holds the window up. When
 * several panes are being searched they take turns.
 *
 * What a bar shows lives here and not in the bar: a pane in a tab that is not on show
 * has no bar, and its search is still there when the tab comes back.
 *
 * Nothing here touches the page, so it can be tested without one.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { PaneId } from "../layout/types.ts";
import type { SearchDirection, SearchProgress } from "../terminal/view.ts";
import { SEARCH_QUERY_DEBOUNCE_MS, SEARCH_REFEED_INTERVAL_MS, SEARCH_SLICE_MS } from "../tuning.ts";

/** What the controller asks of a pane's terminal. A terminal's view is one. */
export interface SearchTarget {
  search(text: string): void;
  searchStep(direction: SearchDirection): void;
  searchPump(budgetMs: number): SearchProgress;
  searchProgress(): SearchProgress;
}

/** What searching needs from its surroundings. */
export interface SearchServices {
  /** The terminal of a pane, if the pane has one in this window. */
  target(pane: PaneId): SearchTarget | undefined;
  /** Runs a task soon, once the page has had a turn at whatever else is waiting. */
  soon(task: () => void): void;
  setTimer(task: () => void, ms: number): number;
  clearTimer(timer: number): void;
  /** The time, in milliseconds, from a clock that never goes back. */
  now(): number;
}

/** A pane's search, as its bar shows it. */
export interface PaneSearch {
  /** Whether the pane's search bar is on show. */
  readonly open: boolean;
  /** What is typed in the bar. It is looked for a moment after the last key. */
  readonly query: string;
  /** Where the keyboard was last in the pane: in the bar, or in the terminal. */
  readonly keyboard: "bar" | "terminal";
  /** How many matches have been found so far. */
  readonly total: number;
  /** Which match the search is at, counted from the newest, which is 0. */
  readonly selected: number | undefined;
  /** There is more of the terminal to look through. */
  readonly running: boolean;
  /** As many were found as are counted, and the looking stopped there. */
  readonly capped: boolean;
}

/** The searches of a window's panes. A pane that was never searched is not in it. */
export interface SearchState {
  readonly panes: Readonly<Record<PaneId, PaneSearch>>;
}

/** The store a window keeps its searches in. */
export type SearchStore = StoreApi<SearchState>;

/** How a search went: for measuring. */
export interface SearchRun {
  /** From the text being given to the terminal to the search having no more to do. */
  readonly completeMs: number | undefined;
  /** The longest the page was kept from doing anything else. */
  readonly maxSliceMs: number;
}

/** A pane's search bar, as far as giving it the keyboard goes. */
export interface SearchBarHandle {
  focus(): void;
}

const CLOSED: PaneSearch = {
  open: false,
  query: "",
  keyboard: "terminal",
  total: 0,
  selected: undefined,
  running: false,
  capped: false,
};

/** Whether there is more for a search to look through. */
function unfinished(progress: SearchProgress): boolean {
  return (progress.status === "running" || progress.status === "feed-required") && !progress.capped;
}

/**
 * What a search bar says beside its field: which match the search is at and how many
 * there are, or that there are none.
 */
export function searchLabel(search: PaneSearch): string {
  if (search.query === "") return "";
  const more = search.capped ? "+" : "";
  if (search.total === 0) return search.running ? "Searching" : "No results";
  if (search.selected === undefined) return `${search.total}${more}`;
  return `${search.selected + 1} of ${search.total}${more}`;
}

/** Runs the searches of a window's panes. */
export class SearchController {
  readonly store: SearchStore = createStore<SearchState>()(() => ({ panes: {} }));
  readonly #services: SearchServices;
  /** The panes with more to look through, in the order of their turns. */
  readonly #queue: PaneId[] = [];
  #scheduled = false;
  /** A pane's wait between its last key and its search beginning. */
  readonly #typing = new Map<PaneId, number>();
  /** A pane's wait before it may be searched again after its last search ended. */
  readonly #resting = new Map<PaneId, number>();
  readonly #rested = new Map<PaneId, number>();
  readonly #runs = new Map<
    PaneId,
    { startedAt: number; completeMs?: number; maxSliceMs: number }
  >();
  readonly #bars = new Map<PaneId, SearchBarHandle>();

  constructor(services: SearchServices) {
    this.#services = services;
  }

  /** A pane's search, or one that is closed for a pane that has none. */
  of(pane: PaneId): PaneSearch {
    return this.store.getState().panes[pane] ?? CLOSED;
  }

  /**
   * Shows a pane's search bar. With `query` it looks for that at once; without, for
   * what the bar held when it was last put away.
   */
  open(pane: PaneId, query?: string): void {
    const search = this.of(pane);
    this.#set(pane, { open: true, keyboard: "bar", query: query ?? search.query });
    if (query !== undefined || !search.open) this.#apply(pane);
  }

  /** Puts a pane's search bar away and ends its search. What was typed is kept. */
  close(pane: PaneId): void {
    if (!this.of(pane).open) return;
    this.#stopWaiting(pane);
    this.#dequeue(pane);
    this.#services.target(pane)?.search("");
    this.#set(pane, {
      open: false,
      keyboard: "terminal",
      total: 0,
      selected: undefined,
      running: false,
      capped: false,
    });
  }

  /** Takes what was typed in a pane's bar. It is looked for once the typing pauses. */
  setQuery(pane: PaneId, query: string): void {
    if (this.of(pane).query === query) return;
    this.#set(pane, { query });
    this.#stopTyping(pane);
    // An emptied field shows no matches, and there is nothing to wait for.
    if (query === "") {
      this.#apply(pane);
      return;
    }
    const { setTimer } = this.#services;
    this.#typing.set(
      pane,
      setTimer(() => {
        this.#typing.delete(pane);
        this.#apply(pane);
      }, SEARCH_QUERY_DEBOUNCE_MS),
    );
  }

  /** Goes to the next match in a direction. */
  step(pane: PaneId, direction: SearchDirection): void {
    if (!this.of(pane).open) return;
    // What was just typed is looked for first: its newest match is gone to as soon
    // as there is one.
    if (this.#typing.has(pane)) {
      this.#stopTyping(pane);
      this.#apply(pane);
      return;
    }
    const target = this.#services.target(pane);
    if (!target) return;
    target.searchStep(direction);
    this.#show(pane, target.searchProgress());
  }

  /** Notes where the keyboard is in a pane, so that it is given back to the same place. */
  setKeyboard(pane: PaneId, keyboard: "bar" | "terminal"): void {
    if (this.store.getState().panes[pane] && this.of(pane).keyboard !== keyboard) {
      this.#set(pane, { keyboard });
    }
  }

  /**
   * Told that a pane's search has caught up with its terminal: the numbers may have
   * changed, and there may be newly printed text to look through.
   */
  touched(pane: PaneId): void {
    const target = this.#services.target(pane);
    if (!target || !this.store.getState().panes[pane]) return;
    const progress = target.searchProgress();
    this.#show(pane, progress);
    if (unfinished(progress)) this.#enqueue(pane, true);
  }

  /** How a pane's last search went. */
  lastRun(pane: PaneId): SearchRun | undefined {
    const run = this.#runs.get(pane);
    return run && { completeMs: run.completeMs, maxSliceMs: run.maxSliceMs };
  }

  // The bars.

  /** Makes a pane's bar known. Returns what forgets it again. */
  registerBar(pane: PaneId, bar: SearchBarHandle): () => void {
    this.#bars.set(pane, bar);
    return () => {
      if (this.#bars.get(pane) === bar) this.#bars.delete(pane);
    };
  }

  /**
   * Gives the keyboard to a pane's search bar, if the bar is on show and the keyboard
   * was there last, or `always`. Returns whether it did.
   */
  focusBar(pane: PaneId, always = false): boolean {
    const search = this.of(pane);
    const bar = this.#bars.get(pane);
    if (!bar || !search.open || (!always && search.keyboard !== "bar")) return false;
    bar.focus();
    return true;
  }

  // The panes there are.

  /** Forgets the searches of panes that are gone. */
  prune(alive: ReadonlySet<PaneId>): void {
    const panes = this.store.getState().panes;
    const gone = Object.keys(panes).filter((pane) => !alive.has(pane));
    if (gone.length === 0) return;
    const kept = { ...panes };
    for (const pane of gone) {
      this.#stopWaiting(pane);
      this.#dequeue(pane);
      this.#runs.delete(pane);
      this.#rested.delete(pane);
      delete kept[pane];
    }
    this.store.setState({ panes: kept });
  }

  /** Forgets every search: the window shows other panes altogether. */
  reset(): void {
    this.prune(new Set());
  }

  // Doing the looking.

  /** Has a pane's terminal look for what its bar holds now. */
  #apply(pane: PaneId): void {
    const { query } = this.of(pane);
    const target = this.#services.target(pane);
    this.#stopWaiting(pane);
    this.#runs.set(pane, { startedAt: this.#services.now(), maxSliceMs: 0 });
    if (!target) return;
    target.search(query);
    const progress = target.searchProgress();
    this.#show(pane, progress);
    if (unfinished(progress)) this.#enqueue(pane, false);
    else this.#dequeue(pane);
  }

  /**
   * Gives a pane a turn at being searched. With `rested`, not before it has rested
   * from its last search: a program that prints without pause would otherwise have its
   * pane searched without pause too.
   */
  #enqueue(pane: PaneId, rested: boolean): void {
    if (this.#queue.includes(pane) || this.#resting.has(pane)) return;
    const { now, setTimer } = this.#services;
    const since = now() - (this.#rested.get(pane) ?? -Infinity);
    if (rested && since < SEARCH_REFEED_INTERVAL_MS) {
      this.#resting.set(
        pane,
        setTimer(() => {
          this.#resting.delete(pane);
          this.#enqueue(pane, false);
        }, SEARCH_REFEED_INTERVAL_MS - since),
      );
      return;
    }
    this.#queue.push(pane);
    this.#schedule();
  }

  #dequeue(pane: PaneId): void {
    const at = this.#queue.indexOf(pane);
    if (at >= 0) this.#queue.splice(at, 1);
  }

  #schedule(): void {
    if (this.#scheduled || this.#queue.length === 0) return;
    this.#scheduled = true;
    this.#services.soon(() => {
      this.#scheduled = false;
      this.#turn();
      this.#schedule();
    });
  }

  /** Searches the pane whose turn it is for one stretch. */
  #turn(): void {
    const pane = this.#queue.shift();
    if (pane === undefined) return;
    const target = this.#services.target(pane);
    if (!target) return;
    const { now } = this.#services;
    const began = now();
    const progress = target.searchPump(SEARCH_SLICE_MS);
    const ended = now();
    const run = this.#runs.get(pane);
    if (run) run.maxSliceMs = Math.max(run.maxSliceMs, ended - began);
    this.#show(pane, progress);
    if (unfinished(progress)) {
      this.#queue.push(pane);
      return;
    }
    // A search that waits for its terminal to settle has not finished, and goes on
    // as soon as the terminal has.
    if (progress.status === "held") return;
    this.#rested.set(pane, ended);
    if (run && run.completeMs === undefined) run.completeMs = ended - run.startedAt;
  }

  #stopTyping(pane: PaneId): void {
    const timer = this.#typing.get(pane);
    if (timer !== undefined) this.#services.clearTimer(timer);
    this.#typing.delete(pane);
  }

  #stopWaiting(pane: PaneId): void {
    this.#stopTyping(pane);
    const timer = this.#resting.get(pane);
    if (timer !== undefined) this.#services.clearTimer(timer);
    this.#resting.delete(pane);
  }

  // The store.

  #show(pane: PaneId, progress: SearchProgress): void {
    const search = this.of(pane);
    const running = unfinished(progress) || progress.status === "held";
    if (
      search.total === progress.total &&
      search.selected === progress.selected &&
      search.running === running &&
      search.capped === progress.capped
    ) {
      return;
    }
    this.#set(pane, {
      total: progress.total,
      selected: progress.selected,
      running,
      capped: progress.capped,
    });
  }

  #set(pane: PaneId, change: Partial<PaneSearch>): void {
    const panes = this.store.getState().panes;
    this.store.setState({ panes: { ...panes, [pane]: { ...this.of(pane), ...change } } });
  }
}
