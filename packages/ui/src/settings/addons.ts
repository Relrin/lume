/**
 * The plugins as the settings window shows them: one page to manage them all, and a
 * page for each, in the order they load in.
 *
 * A plugin's page holds its own settings, where it has any, then what it may do, how
 * it fares and what it did lately. What is installed comes from the core, which says
 * when it changes; nothing here touches the page, so it is tested with plain objects.
 */

import type { SettingsPage } from "./model.ts";

/** A permission a plugin holds or asks for, in words, with a warning where it is risky. */
export interface AddonPermission {
  readonly what: string;
  readonly warning: string | null;
}

/** A plugin another depends on, and the install it uses. */
export interface AddonDependency {
  /** The plugin, by the id its manifest names. */
  readonly id: string;
  readonly optional: boolean;
  /** The install it uses, where one is installed. */
  readonly bound: string | null;
  /** Whether the user bound it to an install of another id. */
  readonly rebound: boolean;
}

/** One line of the audit log: what a plugin did that needed a permission, and how it went. */
export interface AuditEntry {
  readonly time: string;
  readonly capability: string;
  readonly scope: string;
  readonly outcome: string;
}

/** One install, as the settings window shows it. */
export interface AddonEntry {
  /** The id it is installed under. */
  readonly id: string;
  readonly manifestId: string;
  readonly name: string;
  readonly version: string;
  readonly description: string | null;
  readonly enabled: boolean;
  /** Whether it can be used, and if not, why. */
  readonly state: { readonly ready: boolean; readonly reason: string | null };
  /** Its files changed since it was installed, so it is not used. */
  readonly modified: boolean;
  readonly missing: boolean;
  /** Where it was installed from. */
  readonly source: string;
  /** How it came: a folder, an archive, a repository (`git`), or a folder used where
   *  it lies while it is worked on (`link`). */
  readonly sourceKind: string;
  /** The commit it was taken from, for one from a repository. */
  readonly commit: string | null;
  /** Whether it is a folder used where it lies, for a plugin being worked on. */
  readonly dev: boolean;
  readonly dependencies: readonly AddonDependency[];
  readonly permissions: readonly AddonPermission[];
  /** What it may ask for while it runs. */
  readonly later: readonly string[];
  /** The view it is set up in, which its page here shows: one of its pane views. */
  readonly settingsView?: {
    readonly view: string;
    readonly title: string;
    /** The page, as a path in its package. */
    readonly entry: string;
  } | null;
  readonly health: {
    readonly running: boolean;
    readonly starts: number;
    /** How many times it went over its limits in the last minute. */
    readonly breaches: number;
    readonly lastProblem: string | null;
    /** What of its manifest could not be done as it asks. */
    readonly warnings: readonly string[];
  };
  /** The last lines of its log, the oldest first. */
  readonly log: readonly string[];
  /** Its last audit entries, the newest first. */
  readonly audit: readonly AuditEntry[];
}

/** What installing a package would do, once the user has looked at it. */
export interface InstallPreview {
  /** Says which package, when it is installed. */
  readonly token: number;
  /** The id its manifest gives it. */
  readonly id: string;
  /** The id it would be installed under. */
  readonly local: string;
  readonly name: string;
  readonly version: string;
  readonly description: string | null;
  /** A new install, or the version it would replace and how. */
  readonly change: "new" | "newer" | "same" | "older";
  readonly from: string | null;
  /** What it would be able to do that it could not before. */
  readonly permissions: readonly AddonPermission[];
  readonly later: readonly string[];
  readonly warnings: readonly string[];
  readonly leftOut: readonly string[];
  /** Where the package came from. */
  readonly source: string;
  /** The commit that was fetched, for one from a repository. */
  readonly commit: string | null;
}

/** A repository that holds nothing newer than what is installed from it. */
export interface UpdateCurrent {
  readonly current: true;
  readonly version: string;
}

/** Where a package to install is taken from. */
export type InstallFrom =
  | { readonly kind: "folder"; readonly folder: string }
  /** A repository: its address, with `#<ref>` after it for a branch or a tag, and the
   *  folder of it the package is in, if not its top. */
  | { readonly kind: "git"; readonly source: string; readonly path: string | null }
  /** The repository an install was taken from, looked at again. */
  | { readonly kind: "update"; readonly id: string };

/** Where an install came from, in a few words, where that says more than its id. */
export function sourceWords(addon: AddonEntry): string | null {
  if (addon.dev) return `Linked to ${addon.source}`;
  if (addon.sourceKind === "git") {
    const commit = addon.commit === null ? "" : `, at commit ${addon.commit.slice(0, 12)}`;
    return `From ${addon.source}${commit}`;
  }
  return null;
}

/** A package whose id another package is installed under. */
export interface InstallTaken {
  readonly taken: {
    readonly local: string;
    /** Where the one installed came from. */
    readonly existing: string;
    /** An id it could be installed under beside the other. */
    readonly suggestion: string | null;
  };
}

/** What looking at a package to install says. */
export type Inspection = InstallPreview | InstallTaken;

/** The id of the page that manages the plugins. */
export const MANAGE_PAGE = "addons";

/**
 * The view a plugin is set up in, where its page can show it: a plugin that is turned
 * off or cannot be used has no code to answer its view.
 */
export function settingsViewOf(addon: AddonEntry | undefined): AddonEntry["settingsView"] {
  if (addon === undefined || !addon.enabled || !addon.state.ready) return null;
  if (addon.modified || addon.missing) return null;
  return addon.settingsView ?? null;
}

/** The id of a plugin's own page. */
export function addonPage(id: string): string {
  return `addon:${id}`;
}

/** The heading of a page that installing a package would do, in words. */
export function installHeading(preview: InstallPreview): string {
  switch (preview.change) {
    case "new":
      return `Install ${preview.name} ${preview.version}`;
    case "newer":
      return `Update ${preview.name} from ${preview.from ?? "?"} to ${preview.version}`;
    case "same":
      return `Install ${preview.name} ${preview.version} again`;
    case "older":
      return `Go back from ${preview.name} ${preview.from ?? "?"} to ${preview.version}`;
  }
}

/**
 * The pages with the plugins' among them: the page that manages them first under
 * Add-ons, then a page for each plugin in the order they load in, its own settings
 * page where it has one. Pages under Add-ons that are no plugin's stay after them.
 */
export function withAddons(
  pages: readonly SettingsPage[],
  addons: readonly AddonEntry[] | null,
): SettingsPage[] {
  const others = pages.filter((page) => page.group !== "Add-ons");
  const own = pages.filter((page) => page.group === "Add-ons");
  const manage: SettingsPage = {
    id: MANAGE_PAGE,
    label: "Manage add-ons",
    icon: "plugin",
    group: "Add-ons",
    blurb:
      "Plugins installed, in the order they load in. Each can only do what it was allowed when it was installed, or what you allow it later.",
    groups: [],
    manage: true,
  };
  const listed = (addons ?? []).map((addon): SettingsPage => {
    const page = own.find((held) => held.id === addonPage(addon.id));
    return {
      ...(page ?? {
        id: addonPage(addon.id),
        label: addon.name,
        icon: "plugin",
        group: "Add-ons",
        meta: `v${addon.version}`,
        blurb: addon.description ?? "",
        groups: [],
      }),
      addon: addon.id,
    };
  });
  const rest = own.filter((page) => !listed.some((held) => held.id === page.id));
  return [...others, manage, ...listed, ...rest];
}
