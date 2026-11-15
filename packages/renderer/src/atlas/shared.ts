/**
 * Glyph atlases that several panes use at once.
 *
 * Where panes can share textures, every pane showing the same font at the same size
 * draws from one atlas. Such an atlas belongs to nobody in particular, so it is counted:
 * each pane that takes it is a user, and when the last user lets go its pages are freed.
 * Without the count a window that showed many fonts and sizes over its life would keep
 * the pages of all of them.
 */

import type { AtlasSink } from "../types.ts";
import type { GlyphAtlas } from "./atlas.ts";

/** Where a shared atlas keeps its pixels: a sink that can give them up. */
export interface DisposablePages extends AtlasSink {
  dispose(): void;
}

/** A shared atlas and the pages it draws into. */
export interface SharedAtlas<Pages> {
  readonly atlas: GlyphAtlas;
  readonly pages: Pages;
}

interface Entry<Pages> extends SharedAtlas<Pages> {
  users: number;
}

/** The atlases panes share, by font key, each kept for as long as it has a user. */
export class SharedAtlases<Pages extends DisposablePages> {
  readonly #entries = new Map<string, Entry<Pages>>();

  /** How many atlases are alive. */
  get size(): number {
    return this.#entries.size;
  }

  /** How many users the atlas for a key has. Nought when there is no such atlas. */
  users(key: string): number {
    return this.#entries.get(key)?.users ?? 0;
  }

  /**
   * Takes the atlas for a key, making it over new pages if there is none. Every call
   * must be matched by one `release` of the same key.
   */
  acquire(
    key: string,
    pages: () => Pages,
    create: (sink: AtlasSink) => GlyphAtlas,
  ): SharedAtlas<Pages> {
    let entry = this.#entries.get(key);
    if (!entry) {
      const made = pages();
      entry = { atlas: create(made), pages: made, users: 0 };
      this.#entries.set(key, entry);
    }
    entry.users++;
    return entry;
  }

  /**
   * Lets go of the atlas for a key. Its pages are freed with its last user. Releasing a
   * key that has no atlas does nothing: the atlases may all have been freed already.
   */
  release(key: string): void {
    const entry = this.#entries.get(key);
    if (!entry) return;
    entry.users--;
    if (entry.users > 0) return;
    this.#entries.delete(key);
    entry.pages.dispose();
  }

  /** Frees every atlas, whoever still uses it. */
  clear(): void {
    for (const entry of this.#entries.values()) entry.pages.dispose();
    this.#entries.clear();
  }
}
