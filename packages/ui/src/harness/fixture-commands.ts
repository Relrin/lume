/**
 * A few commands and keys for a stand-in core to list: enough for a palette to have
 * something to show and for a key to have something to run.
 */

import type { CommandSpec, KeyBinding } from "../core/types.ts";

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
