/**
 * The pages the settings window draws, and the list they are chosen from.
 *
 * The application's own pages are worked out from the configuration's schema. A page
 * may also be handed over whole: the same data, with a version or a word beside its
 * name and something of its own to show in the column beside its rows. Such pages are
 * listed after the application's, under the heading they name.
 *
 * Nothing here touches the page, so it is tested with plain objects.
 */

import type { SettingsPage } from "./model.ts";

/** The headings pages are listed under, in the order the headings are shown. */
export const PAGE_GROUPS = ["Application", "Workspace", "Add-ons"] as const;

/** One line of the list of pages: a heading, or a page under it. */
export type NavEntry =
  | { readonly kind: "heading"; readonly label: string }
  | { readonly kind: "page"; readonly page: SettingsPage };

/** Holds the pages that do not come from the configuration's schema. */
export class SettingsRegistry {
  #pages: readonly SettingsPage[] = [];
  readonly #listeners = new Set<() => void>();

  /** The pages handed over so far. The same list until it changes. */
  readonly current = (): readonly SettingsPage[] => this.#pages;

  /** Calls `listener` whenever the pages change. Returns how to stop. */
  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  /**
   * Adds a page, in place of one of the same id if there is one. Returns how to take
   * it away again.
   */
  register(page: SettingsPage): () => void {
    this.#pages = [...this.#pages.filter((held) => held.id !== page.id), page];
    this.#changed();
    return () => {
      if (!this.#pages.includes(page)) return;
      this.#pages = this.#pages.filter((held) => held !== page);
      this.#changed();
    };
  }

  #changed(): void {
    for (const listener of this.#listeners) listener();
  }
}

/**
 * Every page the window draws: the application's own, then the ones handed over. A
 * page handed over under the id of one of the application's own is passed over.
 */
export function allPages(
  core: readonly SettingsPage[],
  registered: readonly SettingsPage[],
): SettingsPage[] {
  const taken = new Set(core.map((page) => page.id));
  return [...core, ...registered.filter((page) => !taken.has(page.id))];
}

/**
 * The list of pages as it is shown: each heading that has pages, then its pages in the
 * order they were given. A heading nobody foresaw comes after the ones that were.
 */
export function navOf(pages: readonly SettingsPage[]): NavEntry[] {
  const headings: string[] = PAGE_GROUPS.filter((label) =>
    pages.some((page) => page.group === label),
  );
  for (const page of pages) {
    if (!headings.includes(page.group)) headings.push(page.group);
  }
  return headings.flatMap((label) => [
    { kind: "heading", label } as const,
    ...pages
      .filter((page) => page.group === label)
      .map((page) => ({ kind: "page", page }) as const),
  ]);
}
