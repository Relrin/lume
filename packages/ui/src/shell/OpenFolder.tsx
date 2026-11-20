/**
 * The dialog that opens a folder as a workspace.
 *
 * One field and one list, kept in step: what is typed narrows the folders of the
 * folder it is in, the arrows choose one, Tab goes into it, and Enter opens the one
 * chosen as a workspace named after it. While nothing is typed the workspaces' own
 * folders come first. Browse… hands the choice to the system's folder picker. A
 * folder a workspace already has opens that workspace instead of making another.
 */

import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type KeyboardEvent,
} from "react";

import type { ConfigPlatform, FolderList } from "../core/types.ts";
import { useShell } from "./context.ts";
import { FolderIcon } from "./icons.tsx";
import { Glyph } from "../icons/Glyph.tsx";
import {
  crumbsOf,
  cutPath,
  folderDialog,
  folderName,
  folderView,
  freeName,
  into,
  shownPath,
  upFrom,
  type RecentFolder,
} from "./open-folder.ts";

/** What this machine is called in the dialog, and what it runs. */
const HOST = "This PC";
const SYSTEMS: Readonly<Record<ConfigPlatform, string>> = {
  windows: "Windows",
  macos: "macOS",
  linux: "Linux",
};

/** The system's folder picker, named for what Browse… waits on. */
const PICKERS: Readonly<Record<ConfigPlatform, string>> = {
  windows: "Explorer",
  macos: "Finder",
  linux: "the folder picker",
};

function samePath(one: string, other: string, platform: ConfigPlatform): boolean {
  const plain = (path: string) => path.replace(/[\\/]+$/, "");
  return platform === "windows"
    ? plain(one).toLowerCase() === plain(other).toLowerCase()
    : plain(one) === plain(other);
}

function Dialog({ keyboard, close }: { keyboard: boolean; close: () => void }) {
  const app = useShell();
  const platform = app.config.getState().view?.platform ?? "windows";
  const [typed, setTyped] = useState("");
  const [chosen, setChosen] = useState(0);
  const [moved, setMoved] = useState(false);
  const [hidden, setHidden] = useState(false);
  const [waiting, setWaiting] = useState(false);
  const [listings, setListings] = useState<ReadonlyMap<string, FolderList>>(new Map());
  const [shownDir, setShownDir] = useState("~");
  const [workspaces, setWorkspaces] = useState<readonly { name: string; root: string | null }[]>(
    [],
  );
  const field = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);

  const { dir } = cutPath(typed);

  useEffect(() => {
    let current = true;
    app.client.workspacesList().then(
      (listed) => {
        if (current) setWorkspaces(listed.workspaces);
      },
      () => undefined,
    );
    return () => {
      current = false;
    };
  }, [app]);

  // The folder being typed in is listed once; the list before it stays until it is.
  useEffect(() => {
    if (listings.has(dir)) {
      setShownDir(dir);
      return undefined;
    }
    let current = true;
    app.client.foldersList(dir).then(
      (listing) => {
        if (!current) return;
        setListings((held) => new Map(held).set(dir, listing));
        setShownDir(dir);
      },
      () => undefined,
    );
    return () => {
      current = false;
    };
  }, [app, dir, listings]);

  const listing = listings.get(shownDir) ?? null;
  const recent: RecentFolder[] = workspaces.flatMap((workspace) =>
    workspace.root === null ? [] : [{ workspace: workspace.name, root: workspace.root }],
  );
  const view = folderView({ typed, chosen, moved, hidden }, listing, recent, HOST);
  const separator = listing?.separator ?? (platform === "windows" ? "\\" : "/");
  const home = listing?.home ?? null;
  const show = (path: string) => shownPath(path, home, separator);

  useLayoutEffect(() => {
    if (!view.showChoice) return;
    list.current
      ?.querySelector(`[data-index="${view.chosen}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [view.chosen, view.showChoice]);

  const type = (value: string) => {
    setTyped(value);
    setChosen(0);
    setMoved(false);
  };
  const enter = (path: string) => {
    type(into(path, listing));
    field.current?.focus();
  };
  const up = () => type(upFrom(typed));

  const existing =
    view.target === null
      ? undefined
      : workspaces.find(
          (workspace) =>
            workspace.root !== null && samePath(workspace.root, view.target ?? "", platform),
        );

  const open = () => {
    const target = view.target;
    if (target === null) return;
    close();
    if (existing) {
      app.commands.run("lume.workspace.switch", { name: existing.name });
      return;
    }
    const name = freeName(
      folderName(target),
      workspaces.map((workspace) => workspace.name),
    );
    app.commands.run("lume.workspace.new", { name, root: target });
  };

  const browse = () => {
    if (waiting) return;
    setWaiting(true);
    import("@tauri-apps/plugin-dialog")
      .then(({ open: pick }) =>
        pick({
          directory: true,
          multiple: false,
          title: "Open folder as workspace",
          ...(listing ? { defaultPath: listing.path } : {}),
        }),
      )
      .then(
        (picked) => {
          setWaiting(false);
          if (typeof picked === "string") type(show(picked));
          field.current?.focus();
        },
        () => setWaiting(false),
      );
  };

  const onKey = (event: KeyboardEvent<HTMLInputElement>) => {
    const input = event.currentTarget;
    const atEnd = input.selectionStart === typed.length;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close();
    } else if (event.altKey && event.key === "ArrowUp") {
      event.preventDefault();
      up();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (view.count === 0) return;
      const step = event.key === "ArrowDown" ? 1 : -1;
      setChosen((view.chosen + step + view.count) % view.count);
      setMoved(true);
    } else if (
      (event.key === "Tab" && !event.shiftKey) ||
      (event.key === "ArrowRight" && atEnd && view.count > 0)
    ) {
      const row = view.rows.find((found) => found.kind === "folder" && found.index === view.chosen);
      if (row?.kind === "folder") {
        event.preventDefault();
        enter(row.path);
      }
    } else if (event.key === "Enter") {
      event.preventDefault();
      open();
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "b") {
      event.preventDefault();
      browse();
    } else if (
      event.key === "Backspace" &&
      !event.ctrlKey &&
      /[\\/]$/.test(typed) &&
      atEnd &&
      typed !== "~" + separator
    ) {
      // One press drops the whole last folder.
      event.preventDefault();
      up();
    }
  };

  const crumbs = crumbsOf(listing);
  const { partial } = cutPath(typed);
  const targetName = view.target === null ? "" : folderName(view.target);
  const targetInfo =
    view.target === null
      ? ""
      : existing
        ? `Opens the workspace “${existing.name}” on ${HOST}`
        : `Workspace “${targetName}” on ${HOST}` +
          (view.targetBranch === null ? "" : ` · git ${view.targetBranch}`);
  const activeId = view.showChoice && view.count > 0 ? `open-folder-${view.chosen}` : undefined;

  return (
    <>
      <div className="open-folder-scrim" aria-hidden="true" onPointerDown={close} />
      <div
        className="open-folder"
        role="dialog"
        aria-modal="true"
        aria-labelledby="open-folder-title"
        data-animate={!keyboard || undefined}
      >
        <div className="open-folder-head">
          <h2 id="open-folder-title">Open folder as workspace</h2>
          <span className="open-folder-key">Esc</span>
        </div>
        <div className="open-folder-hosts" role="group" aria-label="Host">
          <button type="button" className="open-folder-host" aria-pressed="true">
            <i />
            {HOST}
            <small>{SYSTEMS[platform]}</small>
          </button>
        </div>
        <div className="open-folder-field">
          <FolderIcon size={15} />
          <span className="open-folder-wrap">
            <span className="open-folder-ghost" aria-hidden="true">
              {view.ghost === "" ? "" : typed}
              <b>{view.ghost}</b>
            </span>
            <input
              ref={field}
              // The dialog takes the keyboard as it opens.
              autoFocus
              value={typed}
              placeholder="Type a path, or pick a folder below or browse"
              role="combobox"
              aria-expanded="true"
              aria-controls="open-folder-list"
              aria-activedescendant={activeId}
              aria-autocomplete="both"
              aria-label="Folder path"
              autoComplete="off"
              spellCheck={false}
              onChange={(event) => type(event.target.value)}
              onKeyDown={onKey}
            />
          </span>
          <button
            type="button"
            className="open-folder-button"
            data-waiting={waiting || undefined}
            title="Open the system folder picker"
            onClick={browse}
          >
            {waiting ? (
              <>
                <span className="open-folder-spin" />
                Waiting for {PICKERS[platform]}…
              </>
            ) : (
              <>
                <Glyph name="folder-open" size={14} />
                Browse…
                <span className="open-folder-key">{platform === "macos" ? "⌘B" : "Ctrl B"}</span>
              </>
            )}
          </button>
        </div>
        <div className="open-folder-crumbs" aria-label="Location">
          {crumbs.map((crumb, at) => (
            <span key={crumb.path} className="open-folder-crumb">
              {at > 0 && <span className="open-folder-sep">{separator}</span>}
              <button type="button" onClick={() => enter(crumb.path)}>
                {crumb.label}
              </button>
            </span>
          ))}
          <span className="open-folder-space" />
          <button
            type="button"
            className="open-folder-toggle"
            aria-pressed={hidden}
            onClick={() => setHidden(!hidden)}
          >
            {hidden ? "Hide hidden folders" : "Show hidden folders"}
          </button>
        </div>
        <div
          ref={list}
          className="open-folder-list"
          id="open-folder-list"
          role="listbox"
          aria-label="Folders"
        >
          {view.rows.map((row, at) =>
            row.kind === "heading" ? (
              <div key={`heading:${at}`} className="open-folder-heading">
                {row.label}
              </div>
            ) : (
              <button
                key={`${row.recent ? "recent" : "folder"}:${row.path}`}
                type="button"
                className="open-folder-row"
                role="option"
                id={`open-folder-${row.index}`}
                data-index={row.index}
                data-recent={row.recent || undefined}
                data-repo={row.branch !== null || undefined}
                aria-selected={view.showChoice && row.index === view.chosen}
                tabIndex={-1}
                onClick={() => enter(row.path)}
              >
                <span className="open-folder-icon">
                  {row.recent ? (
                    <Glyph name="clock" size={15} />
                  ) : row.branch !== null ? (
                    <Glyph name="git-branch" size={15} />
                  ) : (
                    <FolderIcon size={15} />
                  )}
                </span>
                <span className="open-folder-name">
                  {row.match === null ? (
                    row.name
                  ) : (
                    <>
                      {row.name.slice(0, row.match.at)}
                      <mark>{row.name.slice(row.match.at, row.match.at + row.match.length)}</mark>
                      {row.name.slice(row.match.at + row.match.length)}
                    </>
                  )}
                </span>
                {row.recent && <span className="open-folder-path">{show(row.path)}</span>}
                {row.branch !== null && (
                  <span className="open-folder-branch" title="git branch">
                    {row.branch}
                  </span>
                )}
                <span className="open-folder-go" title="Go into">
                  <Glyph name="chevron-right" size={14} />
                </span>
              </button>
            ),
          )}
          {listing !== null && (view.missing || view.count === 0) && (
            <div className="open-folder-empty">
              {view.missing ? (
                <>
                  No folder <b>{show(listing.path)}</b> on {HOST}
                </>
              ) : (
                <>
                  No folder in <b>{show(listing.path)}</b>
                  {partial === "" ? "" : ` starts with “${partial}”`}
                </>
              )}
            </div>
          )}
          {listing?.more === true && !view.missing && (
            <div className="open-folder-more">
              More folders here than are listed: type to find one
            </div>
          )}
        </div>
        <div className="open-folder-foot">
          <div className="open-folder-summary">
            {view.target === null ? (
              <>
                <b data-none="true">Pick a folder</b>
                <small>
                  <span className="open-folder-key">↑</span>{" "}
                  <span className="open-folder-key">↓</span> choose ·{" "}
                  <span className="open-folder-key">Tab</span> go into ·{" "}
                  <span className="open-folder-key">Alt ↑</span> up
                </small>
              </>
            ) : (
              <>
                <b>{show(view.target)}</b>
                <small>{targetInfo}</small>
              </>
            )}
          </div>
          <button type="button" className="open-folder-button" data-quiet="true" onClick={close}>
            Cancel
          </button>
          <button
            type="button"
            className="open-folder-button"
            data-primary="true"
            disabled={view.target === null}
            onClick={open}
          >
            Open
            <span className="open-folder-key">Enter</span>
          </button>
        </div>
      </div>
    </>
  );
}

/** Draws the dialog while it is asked for. */
export function OpenFolder() {
  const app = useShell();
  const open = useSyncExternalStore(
    folderDialog.subscribe,
    () => folderDialog.getState().open,
    () => null,
  );
  if (open === null) return null;
  const close = () => {
    folderDialog.getState().close();
    app.views.focus();
  };
  return <Dialog keyboard={open.keyboard} close={close} />;
}
