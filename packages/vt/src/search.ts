/**
 * Searching a terminal, scrollback included, for a piece of text.
 *
 * The library does the searching and leaves the timing to its caller. A search works on
 * copies of the terminal's pages: `feed` hands it what it has not seen and brings it up
 * to date with output, and `tick` searches what it was handed. `pump` runs the two for a
 * given time and returns, so a long scrollback is searched in slices between frames.
 *
 * Matching is by bytes, except that ASCII letters match in either case. There are no
 * patterns.
 *
 * A match is reported as two references into the terminal's pages, which stay good only
 * until the terminal next changes. They are turned into rows and columns straight after
 * a feed and nothing here keeps one.
 *
 * Which match the search is at is kept here, not by the library. The library lists the
 * matches from the newest to the oldest, except while the rows on screen lie across two
 * of its pages: then it lists the matches of the older of the two before those of the
 * newer, and steps through them in that order. Here the list is read in the order it
 * should have, and the match the search is at is followed by a reference that moves
 * with the terminal, so it is still the same text after more is printed, after old
 * lines are dropped and after the terminal is resized.
 */

import { cellReader } from "./cell.ts";
import { INVALID_VALUE, NO_VALUE, OUT_OF_SPACE } from "./error.ts";
import { HIGHLIGHT_STRIDE, HighlightKind, type Highlights } from "./frame.ts";
import type { Core } from "./module.ts";
import type { Terminal } from "./terminal.ts";

/** What a search can do next. */
export type SearchStatus =
  /** There is copied text left to search: `tick` makes progress. */
  | "running"
  /** It has searched everything it was handed and needs a `feed`. */
  | "feed-required"
  /** It has seen everything the terminal held at the last feed. */
  | "complete";

/** The matches the list of them holds room for at first. It grows to what is needed. */
const MATCHES_MIN = 64;

/** The quads a list of highlights holds room for at first. */
const HIGHLIGHTS_MIN = 64;

/** An empty list of highlights, to be filled by `Search.readViewport`. */
export function newHighlights(): Highlights {
  return { data: new Int32Array(HIGHLIGHTS_MIN * HIGHLIGHT_STRIDE), count: 0 };
}

/**
 * How the library's list of every match is laid out: `older` matches of the older page
 * under the screen come first, then `newer` matches of the newest page, then the rest.
 * When the list is in order both are zero.
 */
interface Order {
  readonly count: number;
  readonly older: number;
  readonly newer: number;
}

/** A search of one terminal. Call `free()` when done with it. */
export class Search {
  readonly #terminal: Terminal;
  readonly #core: Core;
  #search = 0;
  /** Follows the first cell of the match the search is at. */
  #tracked = 0;
  /** Whether the search is at a match. The reference is kept for the next one. */
  #selecting = false;
  /** Which match the search is at, as last worked out, and whether that still holds. */
  #index: number | undefined;
  #indexStale = false;

  // Scratch memory inside the module, allocated once.
  #block: number;
  readonly #blockLen: number;
  readonly #value: number;
  readonly #string: number;
  readonly #buffer: number;
  readonly #selected: number;
  readonly #point: number;
  readonly #place: number;
  /** A list of matches, as the library writes one. */
  #matches = 0;
  #matchesCap = 0;

  // Offsets and values, read from the module's own description of its types.
  readonly #data: Readonly<Record<string, number>>;
  readonly #option: Readonly<Record<string, number>>;
  readonly #statusNames: ReadonlyMap<number, SearchStatus>;
  readonly #tagViewport: number;
  readonly #tagScreen: number;
  readonly #matchSize: number;
  readonly #endAt: number;
  readonly #refSize: number;
  readonly #pointXAt: number;
  readonly #pointYAt: number;
  readonly #placeTagAt: number;
  readonly #placeValueAt: number;
  readonly #bufferPtrAt: number;
  readonly #bufferCapAt: number;
  readonly #bufferLenAt: number;
  readonly #stringPtrAt: number;
  readonly #stringLenAt: number;

  constructor(terminal: Terminal) {
    this.#terminal = terminal;
    const core = terminal.core;
    this.#core = core;
    const m = core.manifest;

    this.#data = m.enumValues("GhosttySearchData");
    this.#option = m.enumValues("GhosttySearchOption");
    const status = m.enumValues("GhosttySearchStatus");
    this.#statusNames = new Map<number, SearchStatus>([
      [status.RUNNING ?? -1, "running"],
      [status.FEED_REQUIRED ?? -1, "feed-required"],
      [status.COMPLETE ?? -1, "complete"],
    ]);
    this.#tagViewport = m.enumValue("GhosttyPointTag", "VIEWPORT");
    this.#tagScreen = m.enumValue("GhosttyPointTag", "SCREEN");
    this.#matchSize = m.struct("GhosttySelection").size;
    this.#endAt = m.offset("GhosttySelection", "end");
    this.#refSize = m.struct("GhosttyGridRef").size;
    this.#pointXAt = m.offset("GhosttyPointCoordinate", "x");
    this.#pointYAt = m.offset("GhosttyPointCoordinate", "y");
    this.#placeTagAt = m.offset("GhosttyPoint", "tag");
    this.#placeValueAt = m.offset("GhosttyPoint", "value");
    this.#bufferPtrAt = m.offset("GhosttySelectionBuffer", "ptr");
    this.#bufferCapAt = m.offset("GhosttySelectionBuffer", "cap");
    this.#bufferLenAt = m.offset("GhosttySelectionBuffer", "len");
    this.#stringPtrAt = m.offset("GhosttyString", "ptr");
    this.#stringLenAt = m.offset("GhosttyString", "len");

    // One allocation in 8-byte slots, as the cell reader's is.
    const align8 = (n: number) => (n + 7) & ~7;
    const sizes = [
      8,
      m.struct("GhosttyString").size,
      m.struct("GhosttySelectionBuffer").size,
      this.#refSize,
      m.struct("GhosttyPointCoordinate").size,
      m.struct("GhosttyPoint").size,
    ];
    this.#blockLen = sizes.reduce((sum, size) => sum + align8(size), 0);
    this.#block = core.allocZeroed(this.#blockLen);
    let offset = 0;
    const [valueAt, stringAt, bufferAt, selectedAt, pointAt, placeAt] = sizes.map((size) => {
      const at = this.#block + offset;
      offset += align8(size);
      return at;
    }) as [number, number, number, number, number, number];
    this.#value = valueAt;
    this.#string = stringAt;
    this.#buffer = bufferAt;
    this.#selected = selectedAt;
    this.#point = pointAt;
    this.#place = placeAt;

    try {
      this.#search = core.newHandle("ghostty_search_new", (out) =>
        core.x.ghostty_search_new(0, out, terminal.handle),
      );
    } catch (err) {
      this.free();
      throw err;
    }
  }

  /**
   * Whether there is anything to call: the search is not freed and its terminal is still
   * there. A search outlives its terminal without harm and then finds nothing.
   */
  get #live(): boolean {
    return this.#search !== 0 && this.#terminal.handle !== 0;
  }

  /**
   * Look for `text` from now on. A new text drops what was found and starts again, at
   * no match; the same text, compared as matching compares, keeps it. An empty text
   * ends the search.
   */
  setNeedle(text: string): void {
    if (!this.#live) return;
    const core = this.#core;
    const before = this.total();
    let value = 0;
    if (text.length > 0) {
      // The bytes go in the shared staging buffer and their description in this
      // search's own memory: the two must not share an address.
      const { ptr, len } = core.stageText(text);
      const view = core.view();
      view.setUint32(this.#string + this.#stringPtrAt, ptr, true);
      view.setUint32(this.#string + this.#stringLenAt, len, true);
      value = this.#string;
    }
    core.check(
      core.x.ghostty_search_set(this.#search, this.#option.NEEDLE ?? 0, value),
      "ghostty_search_set",
    );
    // The library keeps its results for a text it counts as the same one.
    if (text.length === 0 || this.total() !== before || before === 0) this.#selecting = false;
    this.#indexStale = true;
  }

  /** What the search can do next. */
  status(): SearchStatus {
    if (!this.#live || !this.#get("STATUS", this.#value)) return "complete";
    return this.#statusOf(this.#core.view().getInt32(this.#value, true));
  }

  /**
   * Bring the search up to date with the terminal: what was written since the last
   * feed, where the viewport is now, and the next piece of scrollback if the search is
   * waiting for one. Nothing else tells a search that its terminal changed.
   */
  feed(): void {
    if (!this.#live) return;
    this.#core.check(this.#core.x.ghostty_search_feed(this.#search), "ghostty_search_feed");
    this.#indexStale = true;
  }

  /**
   * Search for about `budgetMs`, feeding when the search asks for more, and say where
   * that left it. A search that has seen everything is left alone: `feed` is what tells
   * it that there is more.
   *
   * Once `limit` matches are found the search is left where it is, still running: what
   * a search costs to keep up to date, in time and in memory, grows with what it has
   * found, and a single letter can match a scrollback millions of times.
   */
  pump(budgetMs: number, limit: number, now: () => number = () => performance.now()): SearchStatus {
    if (!this.#live) return "complete";
    const core = this.#core;
    const until = now() + budgetMs;
    let status = this.status();
    for (;;) {
      if (status === "complete" || this.total() >= limit) return status;
      if (status === "feed-required") {
        this.feed();
        status = this.status();
      } else {
        const result = core.x.ghostty_search_tick(this.#search, this.#value);
        if (result === INVALID_VALUE) return "complete";
        core.check(result, "ghostty_search_tick");
        status = this.#statusOf(core.view().getInt32(this.#value, true));
        this.#indexStale = true;
      }
      if (now() >= until) return status;
    }
  }

  /** How many matches have been found so far on the screen in use. */
  total(): number {
    if (!this.#live || !this.#get("TOTAL_MATCHES", this.#value)) return 0;
    return this.#core.view().getUint32(this.#value, true);
  }

  /**
   * Which match the search is at, counted from the newest, which is 0. A match found
   * later below it moves this up by one, so it is asked for when shown, not kept.
   * Nothing while the search is at no match, or while the one it was at is not among
   * those found: a search that starts again after a resize finds it again.
   */
  selectedIndex(): number | undefined {
    if (!this.#live || !this.#selecting) return undefined;
    if (this.#indexStale) {
      this.#indexStale = false;
      this.#index = this.#locate(this.#order(this.#everyMatch()));
    }
    return this.#index;
  }

  /**
   * Go to the match before this one in the text, which is the next older one, or to the
   * newest when the search is at none. After the oldest comes the newest again. Returns
   * whether the search is at a match now. The viewport is not moved.
   */
  next(): boolean {
    return this.#step(1);
  }

  /** Go to the next newer match, or to the oldest when the search is at none. */
  previous(): boolean {
    return this.#step(-1);
  }

  /**
   * The row the match the search is at starts on, counted from the oldest line as
   * `Scrollbar.offset` is.
   */
  selectedRow(): number | undefined {
    if (!this.#live || !this.#selecting || this.#tracked === 0) return undefined;
    const result = this.#core.x.ghostty_tracked_grid_ref_point(
      this.#tracked,
      this.#tagScreen,
      this.#point,
    );
    if (result === NO_VALUE || result === INVALID_VALUE) return undefined;
    this.#core.check(result, "ghostty_tracked_grid_ref_point");
    return this.#core.view().getUint32(this.#point + this.#pointYAt, true);
  }

  /**
   * The matches in the viewport, as runs of cells on one row each. A match that wraps
   * onto the next row is two runs, and one that begins above the viewport or ends below
   * it is cut at the edge.
   *
   * This reads what the last feed found, and the references in it are good only until
   * the terminal changes: call it straight after `feed()`.
   */
  readViewport(into: Highlights): void {
    into.count = 0;
    if (!this.#live) return;
    const core = this.#core;
    const { x } = core;
    const terminal = this.#terminal.handle;
    const { cols, rows } = this.#terminal;
    const found = this.#list("VIEWPORT_MATCHES");
    if (found === 0 || cols === 0 || rows === 0) return;

    // The match the search is at is told from the others by where it starts.
    const hasSelected = this.#snapshot();

    for (let index = 0; index < found; index++) {
      const match = this.#matches + index * this.#matchSize;

      // A match that begins above the viewport has no place in it to begin at. One that
      // ends in it is shown from the first cell.
      let startCol = 0;
      let startRow = 0;
      const started = x.ghostty_terminal_point_from_grid_ref(
        terminal,
        match,
        this.#tagViewport,
        this.#point,
      );
      if (started !== NO_VALUE) {
        core.check(started, "ghostty_terminal_point_from_grid_ref");
        const view = core.view();
        startCol = view.getUint16(this.#point + this.#pointXAt, true);
        startRow = view.getUint32(this.#point + this.#pointYAt, true);
        if (startRow >= rows) continue;
      }

      let endCol = cols - 1;
      let endRow = rows - 1;
      const ended = x.ghostty_terminal_point_from_grid_ref(
        terminal,
        match + this.#endAt,
        this.#tagViewport,
        this.#point,
      );
      if (ended === NO_VALUE) {
        // Wholly above the viewport.
        if (started === NO_VALUE) continue;
      } else {
        core.check(ended, "ghostty_terminal_point_from_grid_ref");
        const view = core.view();
        const row = view.getUint32(this.#point + this.#pointYAt, true);
        if (row < rows) {
          endRow = row;
          endCol = Math.min(view.getUint16(this.#point + this.#pointXAt, true), cols - 1);
        }
      }
      if (endRow < startRow || (endRow === startRow && endCol < startCol)) continue;

      const active = hasSelected && this.#sameRef(match, this.#selected);
      const kind = active ? HighlightKind.Active : HighlightKind.Match;
      for (let row = startRow; row <= endRow; row++) {
        const first = row === startRow ? Math.min(startCol, cols - 1) : 0;
        const last = row === endRow ? endCol : cols - 1;
        push(into, row, first, last, kind);
      }
    }
  }

  /** Release the search. The object must not be used afterwards. */
  free(): void {
    const { x } = this.#core;
    if (this.#tracked !== 0) {
      // The terminal may already be gone, which the library allows for.
      x.ghostty_tracked_grid_ref_free(this.#tracked);
      this.#tracked = 0;
    }
    if (this.#search !== 0) {
      x.ghostty_search_free(this.#search);
      this.#search = 0;
    }
    if (this.#matches !== 0) {
      this.#core.free(this.#matches, this.#matchesCap * this.#matchSize);
      this.#matches = 0;
      this.#matchesCap = 0;
    }
    if (this.#block !== 0) {
      this.#core.free(this.#block, this.#blockLen);
      this.#block = 0;
    }
  }

  #statusOf(value: number): SearchStatus {
    return this.#statusNames.get(value) ?? "complete";
  }

  /** Read one value of the search. False when there is none. */
  #get(key: string, out: number): boolean {
    const result = this.#core.x.ghostty_search_get(this.#search, this.#data[key] ?? 0, out);
    if (result === NO_VALUE || result === INVALID_VALUE) return false;
    this.#core.check(result, "ghostty_search_get");
    return true;
  }

  // Which match the search is at.

  /** Go one match on, towards older ones for 1 and newer ones for -1. */
  #step(by: 1 | -1): boolean {
    if (!this.#live) return false;
    // The list is read as the terminal is now, not as it was at the last feed.
    this.feed();
    const order = this.#order(this.#everyMatch());
    if (order.count === 0) {
      this.#selecting = false;
      return false;
    }
    const at = this.#locate(order);
    const to =
      at === undefined ? (by === 1 ? 0 : order.count - 1) : (at + by + order.count) % order.count;
    if (!this.#follow(this.#matches + this.#libraryIndex(order, to) * this.#matchSize)) {
      return false;
    }
    this.#selecting = true;
    this.#index = to;
    this.#indexStale = false;
    return true;
  }

  /** Have every match written into `#matches`. Returns how many there are. */
  #everyMatch(): number {
    return this.#live ? this.#list("MATCHES") : 0;
  }

  /**
   * How the list in `#matches`, which holds `count` matches, is laid out.
   *
   * A match belongs to the page its reference names. The newest page is the one the
   * bottom row of the screen is on, so the matches that are out of place are those
   * that come before the first match of that page.
   */
  #order(count: number): Order {
    if (count === 0) return { count, older: 0, newer: 0 };
    const core = this.#core;
    const bottom = cellReader(core).ref(
      this.#terminal.handle,
      "active",
      0,
      this.#terminal.rows - 1,
    );
    const words = core.u32();
    const newest = words[bottom >>> 2]!;
    const step = this.#matchSize >>> 2;
    const first = this.#matches >>> 2;
    let older = 0;
    while (older < count && words[first + older * step] !== newest) older++;
    // None of the newest page, or they come first: the list is in order.
    if (older === 0 || older === count) return { count, older: 0, newer: 0 };
    let newer = 0;
    while (older + newer < count && words[first + (older + newer) * step] === newest) newer++;
    return { count, older, newer };
  }

  /** Where in the library's list the match is that is `index` from the newest. */
  #libraryIndex(order: Order, index: number): number {
    if (index < order.newer) return order.older + index;
    if (index < order.older + order.newer) return index - order.newer;
    return index;
  }

  /**
   * Which match the search is at, counted from the newest, given how the list in
   * `#matches` is laid out. Nothing when it is at none, or the one it was at is not in
   * the list.
   */
  #locate(order: Order): number | undefined {
    if (!this.#selecting || !this.#snapshot()) return undefined;
    for (let at = 0; at < order.count; at++) {
      if (this.#sameRef(this.#matches + at * this.#matchSize, this.#selected)) {
        if (at < order.older) return at + order.newer;
        if (at < order.older + order.newer) return at - order.older;
        return at;
      }
    }
    return undefined;
  }

  /**
   * Write where the match the search is at starts into `#selected`, as it is now.
   * False when the search is at none, or its place is gone from the terminal.
   */
  #snapshot(): boolean {
    if (!this.#selecting || this.#tracked === 0) return false;
    const result = this.#core.x.ghostty_tracked_grid_ref_snapshot(this.#tracked, this.#selected);
    if (result === NO_VALUE || result === INVALID_VALUE) return false;
    this.#core.check(result, "ghostty_tracked_grid_ref_snapshot");
    return true;
  }

  /** Follow the first cell of the match at `match` from now on. False if it has no place. */
  #follow(match: number): boolean {
    const core = this.#core;
    const { x } = core;
    const terminal = this.#terminal.handle;
    const placed = x.ghostty_terminal_point_from_grid_ref(
      terminal,
      match,
      this.#tagScreen,
      this.#point,
    );
    if (placed === NO_VALUE) return false;
    core.check(placed, "ghostty_terminal_point_from_grid_ref");

    // A point is passed by value in C; the export takes the address of a copy.
    const view = core.view();
    const value = this.#place + this.#placeValueAt;
    view.setInt32(this.#place + this.#placeTagAt, this.#tagScreen, true);
    view.setUint16(
      value + this.#pointXAt,
      view.getUint16(this.#point + this.#pointXAt, true),
      true,
    );
    view.setUint32(
      value + this.#pointYAt,
      view.getUint32(this.#point + this.#pointYAt, true),
      true,
    );
    if (this.#tracked === 0) {
      this.#tracked = core.newHandle("ghostty_terminal_grid_ref_track", (out) =>
        x.ghostty_terminal_grid_ref_track(terminal, this.#place, out),
      );
    } else {
      core.check(
        x.ghostty_tracked_grid_ref_set(this.#tracked, terminal, this.#place),
        "ghostty_tracked_grid_ref_set",
      );
    }
    return true;
  }

  /** Whether the references at two addresses name the same cell. */
  #sameRef(a: number, b: number): boolean {
    const bytes = this.#core.u8();
    for (let at = 0; at < this.#refSize; at++) {
      if (bytes[a + at] !== bytes[b + at]) return false;
    }
    return true;
  }

  // The library's lists.

  /**
   * Have the library write a list of matches into `#matches`, making room first if
   * there are more than fit. Returns how many there are.
   */
  #list(which: "MATCHES" | "VIEWPORT_MATCHES"): number {
    const core = this.#core;
    const key = this.#data[which] ?? 0;
    if (this.#matchesCap === 0) this.#room(MATCHES_MIN);
    for (let attempt = 0; ; attempt++) {
      const view = core.view();
      view.setUint32(this.#buffer + this.#bufferPtrAt, this.#matches, true);
      view.setUint32(this.#buffer + this.#bufferCapAt, this.#matchesCap, true);
      view.setUint32(this.#buffer + this.#bufferLenAt, 0, true);
      const result = core.x.ghostty_search_get(this.#search, key, this.#buffer);
      const len = core.view().getUint32(this.#buffer + this.#bufferLenAt, true);
      if (result === OUT_OF_SPACE && attempt === 0) {
        // `len` holds how many there are.
        this.#room(Math.max(len, this.#matchesCap * 2));
        continue;
      }
      if (result === NO_VALUE || result === INVALID_VALUE) return 0;
      core.check(result, "ghostty_search_get");
      return len;
    }
  }

  /** Make `#matches` hold `count` matches. What it held is not kept. */
  #room(count: number): void {
    const core = this.#core;
    if (this.#matches !== 0) core.free(this.#matches, this.#matchesCap * this.#matchSize);
    this.#matches = 0;
    this.#matchesCap = 0;
    this.#matches = core.alloc(count * this.#matchSize);
    this.#matchesCap = count;
  }
}

/** Add one run to a list of highlights, making room if it is full. */
function push(into: Highlights, row: number, first: number, last: number, kind: number): void {
  const at = into.count * HIGHLIGHT_STRIDE;
  if (at + HIGHLIGHT_STRIDE > into.data.length) {
    const grown = new Int32Array(into.data.length * 2);
    grown.set(into.data);
    into.data = grown;
  }
  into.data[at] = row;
  into.data[at + 1] = first;
  into.data[at + 2] = last;
  into.data[at + 3] = kind;
  into.count++;
}
