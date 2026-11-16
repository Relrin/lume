/**
 * The titles programs give their terminals, kept by pane.
 *
 * A program may change its title many times a second. Whoever shows a title is told
 * of each change and writes it where it is shown, so a title never passes through the
 * component tree.
 */

import type { PaneId } from "../layout/types.ts";

/** The titles of the panes' programs. */
export class PaneTitles {
  readonly #titles = new Map<PaneId, string>();
  readonly #listeners = new Set<(pane: PaneId) => void>();

  /** The title of a pane's program, or nothing if it has set none. */
  get(pane: PaneId): string | undefined {
    return this.#titles.get(pane);
  }

  /** Every pane's title. The same map as long as nothing changes it. */
  all(): ReadonlyMap<PaneId, string> {
    return this.#titles;
  }

  /** Records the title a pane's program set. An empty title is no title. */
  set(pane: PaneId, title: string): void {
    if ((this.#titles.get(pane) ?? "") === title) return;
    if (title === "") this.#titles.delete(pane);
    else this.#titles.set(pane, title);
    this.#changed(pane);
  }

  /** Forgets a pane that has closed. */
  forget(pane: PaneId): void {
    if (this.#titles.delete(pane)) this.#changed(pane);
  }

  /** Forgets every pane: the window shows another workspace's panes now. */
  clear(): void {
    const panes = [...this.#titles.keys()];
    this.#titles.clear();
    for (const pane of panes) this.#changed(pane);
  }

  /** Calls `listener` with the pane whose title changed. Returns how to stop. */
  subscribe(listener: (pane: PaneId) => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  #changed(pane: PaneId): void {
    for (const listener of [...this.#listeners]) listener(pane);
  }
}
