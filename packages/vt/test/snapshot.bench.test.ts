/**
 * How long this build takes to load snapshots the native build saved with deep
 * scrollback. A measurement, not a check: it runs only when `LUME_SNAPSHOT_DIR` names a
 * folder holding `snapshot-<lines>.bin` files, which
 * `LUME_SNAPSHOT_OUT=<folder> cargo test -p lume-vt --release --test hydrate -- --ignored`
 * writes.
 */

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import type { Terminal } from "../src/index.ts";
import { loadVt } from "./support.ts";

const dir = process.env.LUME_SNAPSHOT_DIR;

describe.skipIf(!dir)("loading native snapshots", () => {
  it.each([1_000, 10_000, 100_000])("of %i lines", async (lines) => {
    const file = join(dir ?? "", `snapshot-${lines}.bin`);
    if (!existsSync(file)) return;
    const bytes = new Uint8Array(readFileSync(file));
    const vt = await loadVt();
    // The first load also warms the module; the next ones are what a view pays.
    vt.loadTerminal(bytes).free();
    const times: number[] = [];
    for (let i = 0; i < 5; i++) {
      const started = performance.now();
      const terminal = vt.loadTerminal(bytes);
      times.push(performance.now() - started);
      expect(terminal.cols).toBe(120);
      terminal.free();
    }
    times.sort((a, b) => a - b);
    console.log(
      `${lines} lines, ${bytes.length} bytes: median ${times[2]!.toFixed(1)} ms, ` +
        `slowest ${times[4]!.toFixed(1)} ms`,
    );

    // What a view holds with all of it, and with only the newest 1000 lines.
    for (const historyLines of [undefined, 1_000]) {
      const terminal = vt.loadTerminal(bytes, historyLines ? { historyLines } : {});
      console.log(
        `  restored ${historyLines ?? "all"}: ${terminal.scrollbar().total} rows, ` +
          `${residentBytes(terminal)} bytes resident`,
      );
      terminal.free();
    }
  });
});

/** The memory a terminal's screens hold, as the library counts it. */
function residentBytes(terminal: Terminal): number {
  const { core } = terminal;
  const m = core.manifest;
  const usage = m.struct("GhosttyTerminalMemoryUsage");
  const at = core.allocZeroed(usage.size);
  try {
    core.x.ghostty_sized_init(at, usage.size);
    terminal.getData("MEMORY_USAGE", at);
    const view = core.view();
    const field = (name: string) =>
      Number(view.getBigUint64(at + m.offset("GhosttyTerminalMemoryUsage", name), true));
    return field("primary_resident_bytes") + field("alternate_resident_bytes");
  } finally {
    core.free(at, usage.size);
  }
}
