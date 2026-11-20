/**
 * What the dialog that opens a folder as a workspace works with: the folder a typed
 * path is in and what is typed after it, the folders to list, which of them is chosen,
 * and the workspace a folder becomes.
 *
 * The field and the list stay in step: what is typed narrows the folders of the
 * folder it is in, the arrows choose among them, and Tab goes into the one chosen. A
 * path may begin with `~`, the home folder, and is shown so wherever it is in it.
 * Nothing here touches the page, so all of it is tested with plain objects.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { FolderList } from "../core/types.ts";

/** A workspace that has a folder: it is what was opened lately. */
export interface RecentFolder {
  readonly workspace: string;
  /** The folder, as the core keeps it. */
  readonly root: string;
}

/** A typed path cut where the folder it is in ends. */
export interface TypedPath {
  /** The folder whose folders are listed: `~` for the home folder when none is typed. */
  readonly dir: string;
  /** What is typed after it, which narrows its folders. */
  readonly partial: string;
}

/** Whether a character separates the folders of a path. */
function isSeparator(character: string | undefined): boolean {
  return character === "/" || character === "\\";
}

/** The folder a typed path is in, and what is typed after it. */
export function cutPath(typed: string): TypedPath {
  if (typed === "~") return { dir: "~", partial: "" };
  let at = typed.length - 1;
  while (at >= 0 && !isSeparator(typed[at])) at--;
  if (at < 0) return { dir: "~", partial: typed };
  // A root keeps its separator: `C:\` and `/` are folders, `C:` and `` are not.
  const dir = typed.slice(0, at + 1);
  const kept = dir.length > 1 && !/^[A-Za-z]:[\\/]$/.test(dir) ? dir.slice(0, -1) : dir;
  return { dir: kept === "" ? "/" : kept, partial: typed.slice(at + 1) };
}

/** A folder's path with the home folder written as `~`, as the dialog shows it. */
export function shownPath(path: string, home: string | null, separator: string): string {
  if (home === null || home === "") return path;
  const same = (one: string, other: string) =>
    separator === "\\" ? one.toLowerCase() === other.toLowerCase() : one === other;
  if (same(path, home)) return "~";
  const prefix = home.endsWith(separator) ? home : home + separator;
  if (path.length > prefix.length && same(path.slice(0, prefix.length), prefix)) {
    return `~${separator}${path.slice(prefix.length)}`;
  }
  return path;
}

/** A path and a folder in it, joined. */
export function joinFolder(dir: string, name: string, separator: string): string {
  return isSeparator(dir.at(-1)) ? dir + name : dir + separator + name;
}

/** The last folder of a path, which is what a workspace opened on it is called. */
export function folderName(path: string): string {
  const parts = path.split(/[\\/]/).filter((part) => part !== "");
  const last = parts.at(-1);
  if (last === undefined || last === "~") return "home";
  // A drive alone is called by its letter.
  return /^[A-Za-z]:$/.test(last) ? last.slice(0, 1).toUpperCase() : last;
}

/** A name for a new workspace that no other has: the folder's, then `-2`, `-3`… */
export function freeName(wanted: string, taken: readonly string[]): string {
  if (!taken.includes(wanted)) return wanted;
  for (let number = 2; ; number++) {
    const name = `${wanted}-${number}`;
    if (!taken.includes(name)) return name;
  }
}

/** One line of the list: a heading, or a folder to choose. */
export type FolderRow =
  | { readonly kind: "heading"; readonly label: string }
  | {
      readonly kind: "folder";
      /** Where it stands among the folders to choose, for the arrows. */
      readonly index: number;
      readonly name: string;
      /** The folder, written out in full. */
      readonly path: string;
      readonly recent: boolean;
      readonly branch: string | null;
      /** Where in its name what is typed is found, if it is. */
      readonly match: { readonly at: number; readonly length: number } | null;
    };

/** What the dialog shows for what is typed, the folder listed, and the choice. */
export interface FolderView {
  readonly rows: readonly FolderRow[];
  /** How many folders there are to choose among. */
  readonly count: number;
  /** The folder chosen in the list, if one is. */
  readonly chosen: number;
  /** Whether the choice is shown: once the arrows moved it, or something is typed. */
  readonly showChoice: boolean;
  /** The folder Open opens, written out in full, if there is one. */
  readonly target: string | null;
  readonly targetBranch: string | null;
  /** The rest of the chosen folder's name after what is typed, shown faintly. */
  readonly ghost: string;
  readonly missing: boolean;
}

/** What the dialog has typed and chosen. */
export interface FolderChoice {
  readonly typed: string;
  readonly chosen: number;
  /** Whether the arrows moved the choice since anything was typed. */
  readonly moved: boolean;
  /** Whether hidden folders are listed. */
  readonly hidden: boolean;
}

/**
 * What the dialog shows: the workspaces' folders first while nothing is typed, then
 * the folders of the folder listed that what is typed is found in, those it begins
 * first.
 */
export function folderView(
  choice: FolderChoice,
  listing: FolderList | null,
  recent: readonly RecentFolder[],
  host: string,
): FolderView {
  const { dir, partial } = cutPath(choice.typed);
  const wanted = partial.toLowerCase();
  const separator = listing?.separator ?? "\\";
  const rows: FolderRow[] = [];
  const paths: { path: string; branch: string | null; name: string }[] = [];

  const recentShown = choice.typed === "" ? recent : [];
  if (recentShown.length > 0) {
    rows.push({ kind: "heading", label: `Recent on ${host}` });
    for (const folder of recentShown) {
      rows.push({
        kind: "folder",
        index: paths.length,
        name: folderName(folder.root),
        path: folder.root,
        recent: true,
        branch: null,
        match: null,
      });
      paths.push({ path: folder.root, branch: null, name: folderName(folder.root) });
    }
    rows.push({ kind: "heading", label: "~ · home" });
  }

  const entries = (listing?.missing ? [] : (listing?.entries ?? []))
    // A hidden folder is listed when asked for, or when its name is being typed.
    .filter((entry) => choice.hidden || !entry.hidden || wanted.startsWith("."))
    .filter((entry) => wanted === "" || entry.name.toLowerCase().includes(wanted))
    .map((entry) => ({ entry, begins: entry.name.toLowerCase().startsWith(wanted) }))
    .sort(
      (one, other) =>
        Number(other.begins) - Number(one.begins) || one.entry.name.localeCompare(other.entry.name),
    )
    .map(({ entry }) => entry);
  for (const entry of entries) {
    const path = joinFolder(listing?.path ?? dir, entry.name, separator);
    const at = wanted === "" ? -1 : entry.name.toLowerCase().indexOf(wanted);
    rows.push({
      kind: "folder",
      index: paths.length,
      name: entry.name,
      path,
      recent: false,
      branch: entry.branch,
      match: at < 0 ? null : { at, length: wanted.length },
    });
    paths.push({ path, branch: entry.branch, name: entry.name });
  }

  const count = paths.length;
  const chosen = Math.min(choice.chosen, Math.max(0, count - 1));
  const current = paths[chosen];
  const missing = listing?.missing === true;
  let target: string | null = null;
  let targetBranch: string | null = null;
  if (!missing && listing !== null) {
    if (partial !== "") {
      const exact = entries.find((entry) => entry.name.toLowerCase() === wanted);
      const picked =
        choice.moved || !exact ? current : paths.find((path) => path.name === exact.name);
      target = picked?.path ?? null;
      targetBranch = picked?.branch ?? null;
    } else if (current && (choice.moved || choice.typed === "")) {
      target = current.path;
      targetBranch = current.branch;
    } else {
      target = listing.path;
      targetBranch = listing.branch;
    }
  }
  const ghost =
    current && partial !== "" && current.name.toLowerCase().startsWith(wanted)
      ? current.name.slice(partial.length) + separator
      : "";
  return {
    rows,
    count,
    chosen,
    showChoice: choice.moved || partial !== "" || choice.typed === "",
    target,
    targetBranch,
    ghost,
    missing,
  };
}

/** The parts of the folder listed, each a place to go back to. */
export function crumbsOf(
  listing: FolderList | null,
): readonly { readonly label: string; readonly path: string }[] {
  if (listing === null) return [{ label: "~", path: "~" }];
  const separator = listing.separator;
  const shown = shownPath(listing.path, listing.home, separator);
  const crumbs: { label: string; path: string }[] = [];
  let path = "";
  if (shown.startsWith("/")) {
    path = "/";
    crumbs.push({ label: "/", path });
  }
  for (const part of shown.split(/[\\/]/).filter((piece) => piece !== "")) {
    if (path === "") path = /^[A-Za-z]:$/.test(part) ? part + separator : part;
    else path = joinFolder(path, part, separator);
    crumbs.push({ label: part, path });
  }
  return crumbs;
}

/** The typed path that lists a folder's folders: the folder and a separator after it. */
export function into(path: string, listing: FolderList | null): string {
  const separator = listing?.separator ?? "\\";
  const shown = listing ? shownPath(path, listing.home, separator) : path;
  return isSeparator(shown.at(-1)) ? shown : shown + separator;
}

/**
 * The typed path that lists the folder above the one listed: nothing, which lists
 * the home folder, from the home folder or a root.
 */
export function upFrom(typed: string): string {
  const { dir } = cutPath(typed);
  if (typed === "" || dir === "~") return "";
  const trimmed = dir.replace(/[\\/]$/, "");
  if (trimmed === "" || /^[A-Za-z]:$/.test(trimmed)) return "";
  const parent = cutPath(trimmed).dir;
  if (isSeparator(parent.at(-1))) return parent;
  return parent + (typed.includes("\\") ? "\\" : "/");
}

/** Whether the dialog is open, and how it was opened. */
export interface FolderDialogState {
  /** Whether it is on show, and whether a key opened it, which shows it at once. */
  readonly open: { readonly keyboard: boolean } | null;
  show(keyboard: boolean): void;
  close(): void;
}

/** The dialog of this page: a window has one. */
export const folderDialog: StoreApi<FolderDialogState> = createStore<FolderDialogState>((set) => ({
  open: null,
  show: (keyboard) => set({ open: { keyboard } }),
  close: () => set({ open: null }),
}));
