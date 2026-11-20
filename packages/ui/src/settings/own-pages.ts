/**
 * The pages of the settings window that the window draws itself: the commands with
 * their keys, and what the application is.
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

/** The heading the application's own pages are listed under. */
const APPLICATION = "Application";

/** The heading of the pages about the work in a workspace. */
const WORKSPACE = "Workspace";

const COMMANDS: SettingsPage = {
  id: COMMANDS_PAGE,
  label: "Keybindings",
  icon: "keys",
  group: WORKSPACE,
  blurb: "",
  groups: [],
  own: "commands",
};

const ABOUT: SettingsPage = {
  id: ABOUT_PAGE,
  label: "About",
  icon: "about",
  group: APPLICATION,
  blurb: "",
  groups: [],
  own: "about",
};

/**
 * The pages with the window's own among them: what the application is after the last
 * of the application's pages, and the keys first of the workspace's. A page that
 * already has one of their ids is passed over, as it is for a page handed over.
 */
export function withOwnPages(pages: readonly SettingsPage[]): SettingsPage[] {
  const kept = pages.filter((page) => page.id !== COMMANDS_PAGE && page.id !== ABOUT_PAGE);
  const last = kept.findLastIndex((page) => page.group === APPLICATION);
  const listed = [...kept.slice(0, last + 1), ABOUT, ...kept.slice(last + 1)];
  const first = listed.findIndex((page) => page.group === WORKSPACE);
  const at = first < 0 ? last + 2 : first;
  return [...listed.slice(0, at), COMMANDS, ...listed.slice(at)];
}
