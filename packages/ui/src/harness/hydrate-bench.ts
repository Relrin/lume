/**
 * How long a view takes to start from a session with a deep scrollback, and what it then
 * holds, measured on the real path: the core's session host keeps the session's screen,
 * a new attachment is sent a copy of it, and the page loads the copy into a terminal.
 *
 * The session prints its lines to an attachment that takes them as they come and stays
 * open, so the session stays after its program ends. Each timed start is a second
 * attachment to it, as a view opened on a session another view shows.
 */

import type { Terminal, Vt } from "@lume/vt";

import { WINDOW_HISTORY_DEFAULT_LINES } from "../limits.ts";
import { DEFAULT_APPEARANCE, factsPalette } from "../theme.ts";
import type { Attachment, TransportClient } from "../transport/client.ts";
import type { ClientFacts, TermSize } from "../transport/types.ts";
import { timing, type Timing } from "./timing.ts";
import {
  HYDRATE_COLS,
  HYDRATE_FLOOD_BYTES,
  HYDRATE_ROWS,
  HYDRATE_SAMPLES,
  HYDRATE_TIMEOUT_MS,
} from "./tuning.ts";

/** What starting from a session's screen took, and what the view then held. */
export interface HydrateBench {
  /** The lines of scrollback the copy carried. */
  readonly lines: number;
  /** The size of the copy, in bytes. */
  readonly bytes: number;
  /** From asking to attach to holding a terminal loaded from the copy, with every line. */
  readonly attach: Timing;
  /**
   * The same in its three steps: until the attachment was answered (the host has made
   * the copy by then), until the copy had arrived, and loading it.
   */
  readonly steps: { readonly answered: Timing; readonly arrived: Timing; readonly loaded: Timing };
  /**
   * The copy made and loaded, without its way to the page: the first step and the last
   * of each start, added up. The way to the page grows with the copy at the speed of the
   * application's carrier, and is measured with it.
   */
  readonly madeAndLoaded: Timing;
  /** The memory a view holds, in bytes, as the terminal library counts it. */
  readonly memory: {
    /** With every line of the copy. */
    readonly full: number;
    /** With only the newest lines, as a view that fetches older ones holds. */
    readonly window: number;
    /**
     * The session's own copy and a view's together, over the session's alone: the
     * view holding every line, and the view holding only the newest. The session's
     * terminal holds what the view's holds with every line.
     */
    readonly unpagedRatio: number;
    readonly pagedRatio: number;
  };
}

function facts(size: TermSize): ClientFacts {
  return {
    size,
    cellPx: { w: 8, h: 16 },
    palette: factsPalette(DEFAULT_APPEARANCE),
    scheme: DEFAULT_APPEARANCE.scheme,
    focused: false,
    visible: true,
  };
}

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

/** Takes the program's output as it comes until the program ends. */
function drain(attachment: Attachment): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(
      () => reject(new Error("the lines were not all printed in time")),
      HYDRATE_TIMEOUT_MS,
    );
    attachment.start((frame) => {
      if (frame.kind === "data") {
        attachment.consumed(frame.bytes.length);
      } else if (frame.kind === "exit") {
        window.clearTimeout(timer);
        resolve();
      }
    });
  });
}

/** Attaches once more and loads the copy the attachment starts from, with every line. */
async function startView(
  vt: Vt,
  client: TransportClient,
  backend: string,
  session: string,
  size: TermSize,
): Promise<{ ms: number; steps: [number, number, number]; terminal: Terminal; bytes: Uint8Array }> {
  const started = performance.now();
  const attachment = await client.attach(backend, session, facts(size));
  const answered = performance.now();
  try {
    const copy = await attachment.hydrated();
    if (copy?.form !== "snapshot") {
      throw new Error(`the view was not sent a copy of the screen it can load (${copy?.form})`);
    }
    const arrived = performance.now();
    const terminal = vt.loadTerminal(copy.bytes);
    const loaded = performance.now();
    return {
      ms: loaded - started,
      steps: [answered - started, arrived - answered, loaded - arrived],
      terminal,
      bytes: copy.bytes,
    };
  } finally {
    attachment.detach();
  }
}

/** Times views starting from a session that has printed about ten thousand lines. */
export async function timeHydrate(
  vt: Vt,
  client: TransportClient,
  backend: string,
): Promise<HydrateBench> {
  const size = { cols: HYDRATE_COLS, rows: HYDRATE_ROWS };
  const launcher = {
    type: "shell" as const,
    program: "flood",
    args: [String(HYDRATE_FLOOD_BYTES)],
    login: false,
  };
  const session = await client.spawn(backend, { size, launcher });
  const printing = await client.attach(backend, session, facts(size));
  try {
    await drain(printing);
    // The first start also warms the module and the way to the core.
    const first = await startView(vt, client, backend, session, size);
    first.terminal.free();
    const times: number[] = [];
    const steps: [number[], number[], number[]] = [[], [], []];
    let last: { terminal: Terminal; bytes: Uint8Array } | undefined;
    for (let i = 0; i < HYDRATE_SAMPLES; i++) {
      const view = await startView(vt, client, backend, session, size);
      times.push(view.ms);
      view.steps.forEach((ms, i) => steps[i]!.push(ms));
      last?.terminal.free();
      last = view;
    }
    if (!last) throw new Error("no view was started");
    const lines = last.terminal.scrollbar().total - HYDRATE_ROWS;
    const full = residentBytes(last.terminal);
    last.terminal.free();
    const windowed = vt.loadTerminal(last.bytes, { historyLines: WINDOW_HISTORY_DEFAULT_LINES });
    const window = residentBytes(windowed);
    windowed.free();
    const ratio = (view: number) => Math.round(((full + view) / full) * 100) / 100;
    return {
      lines,
      bytes: last.bytes.length,
      attach: timing(times),
      madeAndLoaded: timing(steps[0].map((answered, i) => answered + (steps[2][i] ?? 0))),
      steps: {
        answered: timing(steps[0]),
        arrived: timing(steps[1]),
        loaded: timing(steps[2]),
      },
      memory: { full, window, unpagedRatio: ratio(full), pagedRatio: ratio(window) },
    };
  } finally {
    printing.detach();
    await client.kill(backend, session).catch(() => undefined);
  }
}
