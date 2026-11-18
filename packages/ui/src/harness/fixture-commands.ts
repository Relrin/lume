/**
 * A few commands and keys for a stand-in core to list: enough for a palette to have
 * something to show and for a key to have something to run.
 */

import type { CommandSpec, DetachedSession, KeyBinding } from "../core/types.ts";

/** Commands of each kind the palette shows: carried out in the core and in the page. */
export const FIXTURE_COMMANDS: readonly CommandSpec[] = [
  { id: "lume.tab.new", title: "New Tab", category: "Tab", runs: "core", palette: true },
  {
    id: "lume.pane.split-right",
    title: "Split Right",
    category: "Pane",
    runs: "core",
    palette: true,
  },
  { id: "lume.pane.close", title: "Close Pane", category: "Pane", runs: "core", palette: true },
  {
    id: "lume.palette.open",
    title: "Command Palette",
    category: "View",
    runs: "page",
    palette: false,
  },
  {
    id: "lume.search.open",
    title: "Find in Terminal",
    category: "Search",
    runs: "page",
    palette: true,
    args: {
      type: "object",
      properties: { query: { type: "string" }, pane: { type: "string" } },
      required: [],
    },
  },
  {
    id: "lume.search.close",
    title: "Close Find",
    category: "Search",
    runs: "page",
    palette: true,
  },
  {
    id: "lume.view.zoom-in",
    title: "Increase Font Size",
    category: "View",
    runs: "page",
    palette: true,
  },
];

/** The keys that run them. */
export const FIXTURE_BINDINGS: readonly KeyBinding[] = [
  { keys: ["ctrl+shift+t"], command: "lume.tab.new", args: null, when: [], source: "default" },
  {
    keys: ["ctrl+shift+p"],
    command: "lume.palette.open",
    args: null,
    when: [],
    source: "default",
  },
  {
    keys: ["ctrl+shift+f"],
    command: "lume.search.open",
    args: null,
    when: ["terminal"],
    source: "default",
  },
];

/**
 * The commands about sessions that run with no pane to show them, about a session shown
 * in a second place, and about the session host.
 */
export const SESSION_COMMANDS: readonly CommandSpec[] = [
  {
    id: "lume.session.detached",
    title: "Detached Sessions…",
    category: "Session",
    runs: "page",
    palette: true,
  },
  {
    id: "lume.session.reattach",
    title: "Attach to Session",
    category: "Session",
    runs: "core",
    palette: false,
  },
  {
    id: "lume.session.end",
    title: "End Session",
    category: "Session",
    runs: "core",
    palette: false,
  },
  {
    id: "lume.pane.show-in-new-tab",
    title: "Show Pane in New Tab",
    category: "Pane",
    runs: "core",
    palette: true,
  },
  {
    id: "lume.pane.show-in-new-window",
    title: "Show Pane in New Window",
    category: "Pane",
    runs: "core",
    palette: true,
  },
  {
    id: "lume.app.quit-end-sessions",
    title: "Quit and End Sessions",
    category: "Application",
    runs: "core",
    palette: true,
  },
  {
    id: "lume.host.restart",
    title: "Restart Session Host",
    category: "Application",
    runs: "core",
    palette: true,
  },
];

/** Sessions that run with no pane to show them: one running, one whose program ended. */
export const FIXTURE_DETACHED: readonly DetachedSession[] = [
  {
    backend: "local",
    session: "1f2e3d4c5b6a7980",
    program: "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
    title: "npm run dev",
    cwd: "C:\\Users\\fixture\\code\\site",
    created_unix_ms: 1_763_400_000_000,
    exited: false,
    pid: 4242,
  },
  {
    backend: "local",
    session: "0a1b2c3d4e5f6071",
    program: "C:\\Windows\\System32\\cmd.exe",
    title: null,
    cwd: null,
    created_unix_ms: 1_763_390_000_000,
    exited: true,
    pid: null,
  },
];
