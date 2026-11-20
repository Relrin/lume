/**
 * The pages of the settings window that set nothing: the list of commands with their
 * keys, and what the application is.
 *
 * They are pages like the others, listed with the application's own, and the window
 * draws what is on them itself (see `Commands.tsx` and `About.tsx`). Nothing here
 * touches the page, so it is tested with plain objects.
 */

import type { SettingsPage } from "./model.ts";

/** The id of the page that lists the commands. `lume.app.commands` opens it. */
export const COMMANDS_PAGE = "commands";

/** The id of the page that says what the application is. `lume.app.about` opens it. */
export const ABOUT_PAGE = "about";

const OWN_PAGES: readonly SettingsPage[] = [
  {
    id: COMMANDS_PAGE,
    label: "Commands",
    icon: "commands",
    group: "Application",
    blurb:
      "Everything Lume can be asked to do, by what it acts on, with the keys that do it now. Keys are changed under [keybindings] in config.toml.",
    groups: [],
    own: "commands",
  },
  {
    id: ABOUT_PAGE,
    label: "About",
    icon: "about",
    group: "Application",
    blurb: "",
    groups: [],
    own: "about",
  },
];

/**
 * The pages with the window's own after the application's others. A page that already
 * has one of their ids is passed over, as it is for a page handed over.
 */
export function withOwnPages(pages: readonly SettingsPage[]): SettingsPage[] {
  const kept = pages.filter((page) => !OWN_PAGES.some((own) => own.id === page.id));
  const last = kept.findLastIndex((page) => page.group === "Application");
  return [...kept.slice(0, last + 1), ...OWN_PAGES, ...kept.slice(last + 1)];
}
