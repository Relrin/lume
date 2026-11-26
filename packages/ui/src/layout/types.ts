/**
 * The layout as the core writes it: a workspace's windows, their tabs, the splits and
 * panes of each tab, the panes that float, and the panels docked at a window's edges.
 *
 * The types are generated from the core's own definitions (`crates/lume-layout`) into
 * `../wire/layout.generated.ts` by `cargo xtask types`, so they match the core's JSON
 * field for field; a test fails when the two differ. The core owns the layout; a page
 * keeps a copy, follows the changes it is sent, and asks for changes by sending intents.
 *
 * Only what the page adds of its own is written here.
 */

import type { Node } from "../wire/layout.generated.ts";

export * from "../wire/layout.generated.ts";

/** A split as a node of a tree. */
export type SplitNode = Extract<Node, { type: "split" }>;

/** A pane as a node of a tree. */
export type PaneNode = Extract<Node, { type: "pane" }>;
