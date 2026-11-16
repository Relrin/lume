/**
 * Status items of every kind, for tests and for pictures of the window that must look
 * the same each time.
 */

import type { StatusItem } from "./types.ts";

/** One item of each kind, spread over the top bar's three slots. */
export const STATUS_FIXTURES: readonly StatusItem[] = [
  {
    id: "fixture.branch",
    slot: "topbar.left",
    kind: "icon",
    icon: "branch",
    label: "main",
    show_label: true,
    order: 1,
  },
  {
    id: "fixture.directory",
    slot: "topbar.left",
    kind: "text",
    text: "~/src/lume",
    mono: true,
    order: 2,
    tooltip: "The workspace's folder",
  },
  {
    id: "fixture.waiting",
    slot: "topbar.center",
    kind: "badge",
    text: "3",
    tone: "warning",
  },
  {
    id: "fixture.cpu",
    slot: "topbar.right",
    kind: "meter",
    label: "CPU",
    value: 0.31,
    text: "31%",
    tone: "success",
    order: 1,
  },
  {
    id: "fixture.memory",
    slot: "topbar.right",
    kind: "meter",
    label: "RAM",
    value: 0.35,
    text: "11.2 / 32G",
    tone: "info",
    order: 2,
  },
  {
    id: "fixture.load",
    slot: "topbar.right",
    kind: "sparkline",
    label: "Load",
    points: [0.4, 0.9, 0.7, 1.6, 1.2, 2.1, 1.4, 0.8],
    text: "0.8",
    order: 3,
  },
];
