/**
 * The benchmark suite: how long drawing takes, and how fast and how promptly bytes
 * travel from a session to the screen, measured in the real webview on the real path.
 *
 * Drawing is timed for each renderer tier on a full screen of changing text. The path
 * from a session to the page is timed with sessions whose programs are built into the
 * core and are never the slow part: one that prints as fast as it is read, and one that
 * prints back what is typed. A real shell is timed as well, for the record.
 *
 * The suite only measures. Whether a number is good enough is decided by whoever reads
 * the report, against limits kept outside the page.
 */

import { fullRedrawFrame } from "@lume/renderer/testing";
import type { Vt } from "@lume/vt";

import { SCROLLBACK_DEFAULT_LINES, SCROLLBACK_MAX_BYTES } from "../limits.ts";
import { DEFAULT_APPEARANCE, factsPalette } from "../theme.ts";
import { TauriCarrier } from "../transport/carrier.ts";
import { TransportClient, type Attachment } from "../transport/client.ts";
import type { ClientFacts, Launcher } from "../transport/types.ts";
import { timeCursorFx, type FxBench } from "./fx-bench.ts";
import { timeHydrate, type HydrateBench } from "./hydrate-bench.ts";
import { canvasOf, createPane, drawWhole, fit, loadTestFont, TIERS } from "./panes.ts";
import { nextFrame, sleep, timing, type Timing } from "./timing.ts";
import {
  BENCH_COLS,
  BENCH_ROWS,
  ECHO_BURST_SAMPLES,
  ECHO_FLOOD_SAMPLES,
  ECHO_SHELL_SAMPLES,
  ECHO_SPACED_PAUSE_MS,
  ECHO_SPACED_SAMPLES,
  ECHO_TIMEOUT_MS,
  FLOOD_DRAW_MS,
  FLOOD_TERMINAL_BYTES,
  FLOOD_TRANSPORT_BYTES,
  REDRAW_FRAMES,
  REDRAW_WARMUP_FRAMES,
} from "./tuning.ts";
import { benchTerminal, fullRedrawOutput } from "./workloads.ts";

/** The backend whose sessions run the core's built-in programs. */
const BENCH_BACKEND = "bench";

const MIB = 1024 * 1024;

/** How fast bytes travelled. */
export interface Throughput {
  readonly bytes: number;
  readonly ms: number;
  readonly mibPerSecond: number;
}

/** How one tier drew full screens: until the frame was handed over, and until it was done. */
export interface RedrawTiming {
  readonly draw: Timing;
  readonly finish: Timing;
}

/** What drawing a terminal under a flood looked like. */
export interface FloodDrawing {
  /** The tier that drew. */
  readonly tier: string;
  readonly framesPerSecond: number;
  /** The time between one frame and the next. */
  readonly frameInterval: Timing;
  /** From a key being sent to the first frame drawn after its echo arrived. */
  readonly inputToFrame: Timing;
  /** From a key being sent to its echo arriving, meanwhile. */
  readonly echo: Timing;
  /** How fast the flood itself went while it was being drawn. */
  readonly throughput: Throughput;
}

/** The suite's report. Times are in milliseconds. */
export interface BenchReport {
  /** Whether every measurement was taken. It says nothing of how the numbers turned out. */
  readonly ok: boolean;
  readonly devicePixelRatio: number;
  /** Whether the page was hidden at any point, which slows frames and timers. */
  readonly hidden: boolean;
  /** Full redraws of a 200 by 60 screen, for each tier the webview offers. */
  readonly redraw: Record<string, RedrawTiming>;
  /** Tiers the webview does not offer, and why. */
  readonly unavailable: Record<string, string>;
  /** The cursor's effects drawn over a window-sized canvas, each on its own. */
  readonly cursorFx: FxBench;
  /** The terminal's side of a full redraw: taking the output in, reading the screen out. */
  readonly terminal: { readonly write: Timing; readonly read: Timing } | null;
  /** A flood from the core to this page, with nothing done to the bytes. */
  readonly transport: Throughput | null;
  /** The same flood written into a terminal. */
  readonly intoTerminal: Throughput | null;
  /** Echo of keys typed one straight after another. */
  readonly echoBurst: Timing | null;
  /** Echo of keys typed with a pause before each. */
  readonly echoSpaced: Timing | null;
  readonly flood: FloodDrawing | null;
  /** Echo of keys typed at a real shell, which includes the shell and its terminal. */
  readonly echoShell: Timing | null;
  /** A view starting from a session's screen with ten thousand lines of scrollback. */
  readonly hydrate: HydrateBench | null;
  /** What could not be measured, and why. */
  readonly problems: readonly string[];
}

function throughput(bytes: number, ms: number): Throughput {
  return {
    bytes,
    ms: Math.round(ms),
    mibPerSecond: ms > 0 ? Math.round((bytes / MIB / (ms / 1000)) * 10) / 10 : 0,
  };
}

function facts(): ClientFacts {
  return {
    size: { cols: BENCH_COLS, rows: BENCH_ROWS },
    cellPx: { w: 8, h: 16 },
    palette: factsPalette(DEFAULT_APPEARANCE),
    scheme: DEFAULT_APPEARANCE.scheme,
    focused: true,
    visible: true,
  };
}

function builtIn(program: string, ...args: string[]): Launcher {
  return { type: "shell", program, args, login: false };
}

/** The surfaces renderers draw on, kept out of the page's flow. */
function stage(): HTMLElement {
  const made = document.createElement("div");
  made.className = "harness-stage";
  document.body.append(made);
  return made;
}

function surfaceIn(parent: HTMLElement): HTMLElement {
  const made = document.createElement("div");
  made.className = "harness-surface";
  parent.append(made);
  return made;
}

// Drawing.

async function timeRedraws(
  parent: HTMLElement,
): Promise<Pick<BenchReport, "redraw" | "unavailable">> {
  const frames = [
    fullRedrawFrame(0, BENCH_COLS, BENCH_ROWS),
    fullRedrawFrame(1, BENCH_COLS, BENCH_ROWS),
  ];
  const probe = document.createElement("canvas");
  probe.width = 1;
  probe.height = 1;
  const probeContext = probe.getContext("2d", { willReadFrequently: true });
  const redraw: Record<string, RedrawTiming> = {};
  const unavailable: Record<string, string> = {};

  for (const tier of TIERS) {
    const surface = surfaceIn(parent);
    const pane = await createPane(tier, surface);
    if (!pane.renderer) {
      unavailable[tier] = pane.error ?? "unavailable";
      surface.remove();
      continue;
    }
    fit(pane, frames[0]!, window.devicePixelRatio);
    const draw: number[] = [];
    const finish: number[] = [];
    // A few frames first, so the atlas holds every glyph the workload uses.
    for (let i = -REDRAW_WARMUP_FRAMES; i < REDRAW_FRAMES; i++) {
      const start = performance.now();
      drawWhole(pane, frames[i & 1]!);
      const drawn = performance.now();
      // Reading a pixel back makes the webview wait for the GPU to finish the frame,
      // so this time includes the drawing itself and not only queueing it.
      const canvas = canvasOf(pane);
      if (canvas && probeContext) {
        probeContext.drawImage(canvas, 0, 0, 1, 1, 0, 0, 1, 1);
        probeContext.getImageData(0, 0, 1, 1);
      }
      const finished = performance.now();
      if (i >= 0) {
        draw.push(drawn - start);
        finish.push(finished - start);
      }
    }
    redraw[tier] = { draw: timing(draw), finish: timing(finish) };
    pane.renderer.dispose();
    pane.host.dispose();
    surface.remove();
    await nextFrame();
  }
  return { redraw, unavailable };
}

function timeTerminal(vt: Vt): NonNullable<BenchReport["terminal"]> {
  const { term, state } = benchTerminal(vt, BENCH_COLS, BENCH_ROWS, { lines: 100, bytes: MIB });
  const outputs = [
    fullRedrawOutput(0, BENCH_COLS, BENCH_ROWS),
    fullRedrawOutput(1, BENCH_COLS, BENCH_ROWS),
  ];
  const write: number[] = [];
  const read: number[] = [];
  for (let i = 0; i < REDRAW_FRAMES; i++) {
    const start = performance.now();
    term.write(outputs[i & 1]!);
    const written = performance.now();
    state.update();
    const updated = performance.now();
    state.markDrawn();
    write.push(written - start);
    read.push(updated - written);
  }
  state.free();
  term.free();
  return { write: timing(write), read: timing(read) };
}

// The way from a session to the page.

/** A session of one of the core's built-in programs, attached and ready to start. */
async function builtInSession(
  client: TransportClient,
  launcher: Launcher,
): Promise<{ session: string; attachment: Attachment }> {
  const size = { cols: BENCH_COLS, rows: BENCH_ROWS };
  const session = await client.spawn(BENCH_BACKEND, { size, launcher });
  return { session, attachment: await client.attach(BENCH_BACKEND, session, facts()) };
}

/** Runs a flood of `bytes` to its end, giving every piece to `take`. */
async function flood(
  client: TransportClient,
  bytes: number,
  take: (piece: Uint8Array) => void,
): Promise<Throughput> {
  const { attachment } = await builtInSession(client, builtIn("flood", String(bytes)));
  return new Promise((resolve) => {
    let first: number | undefined;
    let total = 0;
    attachment.start((frame) => {
      if (frame.kind === "data") {
        first ??= performance.now();
        total += frame.bytes.length;
        take(frame.bytes);
        attachment.consumed(frame.bytes.length);
      } else if (frame.kind === "exit") {
        attachment.detach();
        resolve(throughput(total, performance.now() - (first ?? performance.now())));
      }
    });
  });
}

/** Something that echoes: types a key and resolves, with the time, when output comes back. */
interface Echo {
  /** The time from sending a key to output arriving, and the moment it arrived. */
  type(): Promise<{ ms: number; arrivedAt: number }>;
  close(): void;
}

function echoOn(attachment: Attachment, key: Uint8Array): Echo {
  let waiting: ((arrivedAt: number) => void) | undefined;
  attachment.start((frame) => {
    if (frame.kind !== "data") return;
    attachment.consumed(frame.bytes.length);
    const resolve = waiting;
    waiting = undefined;
    resolve?.(performance.now());
  });
  return {
    type() {
      return new Promise((resolve, reject) => {
        const sent = performance.now();
        const timer = setTimeout(() => {
          waiting = undefined;
          reject(new Error("a key was typed and nothing came back"));
        }, ECHO_TIMEOUT_MS);
        waiting = (arrivedAt) => {
          clearTimeout(timer);
          resolve({ ms: arrivedAt - sent, arrivedAt });
        };
        attachment.input(key);
      });
    },
    close: () => attachment.detach(),
  };
}

async function builtInEcho(client: TransportClient): Promise<Echo> {
  const { attachment } = await builtInSession(client, builtIn("echo"));
  return echoOn(attachment, Uint8Array.of(0x78));
}

async function typeMany(echo: Echo, samples: number, pauseMs: number): Promise<Timing> {
  const times: number[] = [];
  for (let i = 0; i < samples; i++) {
    if (pauseMs > 0) await sleep(pauseMs);
    times.push((await echo.type()).ms);
  }
  return timing(times);
}

/**
 * Draws a terminal that a flood is being written into, every frame, and types at an
 * echoing session meanwhile.
 */
async function drawUnderFlood(
  vt: Vt,
  client: TransportClient,
  parent: HTMLElement,
): Promise<FloodDrawing> {
  const surface = surfaceIn(parent);
  const pane = await createPane("auto", surface);
  const renderer = pane.renderer;
  if (!renderer) throw new Error(pane.error ?? "no renderer tier is available");
  const { term, state } = benchTerminal(vt, BENCH_COLS, BENCH_ROWS, {
    lines: SCROLLBACK_DEFAULT_LINES,
    bytes: SCROLLBACK_MAX_BYTES,
  });
  fit(pane, state.update(), window.devicePixelRatio);

  // More than can be printed in the time: the flood is ended, it does not finish.
  const { session, attachment } = await builtInSession(
    client,
    builtIn("flood", String(1024 * 1024 * MIB)),
  );
  let flooded = 0;
  let floodStart: number | undefined;
  attachment.start((frame) => {
    if (frame.kind !== "data") return;
    floodStart ??= performance.now();
    flooded += frame.bytes.length;
    term.write(frame.bytes);
    attachment.consumed(frame.bytes.length);
  });

  const frameTimes: number[] = [];
  /** Keys whose echo has arrived and that wait for the next frame. */
  let awaitingFrame: number[] = [];
  const inputToFrame: number[] = [];
  let drawing = true;
  const drawn = (async () => {
    while (drawing) {
      await nextFrame();
      renderer.draw(state.update());
      state.markDrawn();
      const now = performance.now();
      frameTimes.push(now);
      for (const sent of awaitingFrame) inputToFrame.push(now - sent);
      awaitingFrame = [];
    }
  })();

  const echo = await builtInEcho(client);
  const echoTimes: number[] = [];
  const started = performance.now();
  try {
    for (let i = 0; i < ECHO_FLOOD_SAMPLES || performance.now() - started < FLOOD_DRAW_MS; i++) {
      const { ms, arrivedAt } = await echo.type();
      echoTimes.push(ms);
      awaitingFrame.push(arrivedAt - ms);
      // A person does not type faster than frames are drawn; neither does this.
      await nextFrame();
    }
  } finally {
    drawing = false;
    await drawn;
    echo.close();
    attachment.detach();
    await client.kill(BENCH_BACKEND, session).catch(() => undefined);
  }

  const elapsed = performance.now() - (floodStart ?? started);
  const intervals = frameTimes.slice(1).map((at, i) => at - frameTimes[i]!);
  const span = (frameTimes[frameTimes.length - 1] ?? 0) - (frameTimes[0] ?? 0);
  const tier = renderer.tier;
  state.free();
  term.free();
  renderer.dispose();
  pane.host.dispose();
  surface.remove();
  return {
    tier,
    framesPerSecond: span > 0 ? Math.round((intervals.length / span) * 10000) / 10 : 0,
    frameInterval: timing(intervals),
    inputToFrame: timing(inputToFrame),
    echo: timing(echoTimes),
    throughput: throughput(flooded, elapsed),
  };
}

/** Types at a real shell, once it has gone quiet at its prompt. */
async function shellEcho(client: TransportClient): Promise<Timing> {
  const backend = client.defaultBackend;
  const size = { cols: 80, rows: 24 };
  const session = await client.spawn(backend, { size });
  const attachment = await client.attach(backend, session, { ...facts(), size });
  let last = performance.now();
  let waiting: ((arrivedAt: number) => void) | undefined;
  attachment.start((frame) => {
    if (frame.kind !== "data") return;
    attachment.consumed(frame.bytes.length);
    last = performance.now();
    const resolve = waiting;
    waiting = undefined;
    resolve?.(last);
  });
  try {
    // The shell prints its greeting and its prompt in pieces. Typing starts once it has
    // said nothing for a while, so that what comes back is the echo.
    const deadline = performance.now() + 20_000;
    while (performance.now() - last < 1500 && performance.now() < deadline) await sleep(100);

    const times: number[] = [];
    for (let i = 0; i < ECHO_SHELL_SAMPLES; i++) {
      await sleep(ECHO_SPACED_PAUSE_MS);
      const sent = performance.now();
      const arrived = new Promise<number>((resolve, reject) => {
        const timer = setTimeout(
          () => reject(new Error("the shell did not echo a key")),
          ECHO_TIMEOUT_MS,
        );
        waiting = (at) => {
          clearTimeout(timer);
          resolve(at);
        };
      });
      attachment.input(Uint8Array.of(0x61));
      times.push((await arrived) - sent);
    }
    return timing(times);
  } finally {
    attachment.detach();
    await client.kill(backend, session).catch(() => undefined);
  }
}

/** Takes every measurement and reports them. */
export async function runBench(vt: Vt, progress: (text: string) => void): Promise<BenchReport> {
  const problems: string[] = [];
  let hidden = document.hidden;
  const watch = () => {
    hidden ||= document.hidden;
  };
  document.addEventListener("visibilitychange", watch);

  /** Runs one measurement; a failure is noted and the rest still run. */
  async function measure<T>(what: string, run: () => Promise<T> | T): Promise<T | null> {
    progress(what);
    await nextFrame();
    try {
      return await run();
    } catch (error) {
      problems.push(`${what}: ${error instanceof Error ? error.message : String(error)}`);
      return null;
    }
  }

  await loadTestFont();
  const parent = stage();
  const drawing = await measure("timing full redraws", () => timeRedraws(parent));
  const terminal = await measure("timing the terminal", () => timeTerminal(vt));
  const cursorFx = await measure("timing the cursor's effects", () => timeCursorFx(parent));

  const client = await measure("opening the carrier", () =>
    TransportClient.open(new TauriCarrier()),
  );
  let transport: Throughput | null = null;
  let intoTerminal: Throughput | null = null;
  let echoBurst: Timing | null = null;
  let echoSpaced: Timing | null = null;
  let floodDrawing: FloodDrawing | null = null;
  let echoShell: Timing | null = null;
  let hydrate: HydrateBench | null = null;
  if (client) {
    transport = await measure("flooding the page", () =>
      flood(client, FLOOD_TRANSPORT_BYTES, () => undefined),
    );
    intoTerminal = await measure("flooding a terminal", async () => {
      const { term, state } = benchTerminal(vt, BENCH_COLS, BENCH_ROWS, {
        lines: SCROLLBACK_DEFAULT_LINES,
        bytes: SCROLLBACK_MAX_BYTES,
      });
      try {
        return await flood(client, FLOOD_TERMINAL_BYTES, (piece) => term.write(piece));
      } finally {
        state.free();
        term.free();
      }
    });
    echoBurst = await measure("typing in a burst", async () => {
      const echo = await builtInEcho(client);
      try {
        return await typeMany(echo, ECHO_BURST_SAMPLES, 0);
      } finally {
        echo.close();
      }
    });
    echoSpaced = await measure("typing with pauses", async () => {
      const echo = await builtInEcho(client);
      try {
        return await typeMany(echo, ECHO_SPACED_SAMPLES, ECHO_SPACED_PAUSE_MS);
      } finally {
        echo.close();
      }
    });
    floodDrawing = await measure("drawing under a flood", () => drawUnderFlood(vt, client, parent));
    echoShell = await measure("typing at a shell", () => shellEcho(client));
    hydrate = await measure("starting views from a deep scrollback", () =>
      timeHydrate(vt, client, BENCH_BACKEND),
    );
  }

  document.removeEventListener("visibilitychange", watch);
  parent.remove();
  return {
    ok: problems.length === 0,
    devicePixelRatio: window.devicePixelRatio,
    hidden,
    redraw: drawing?.redraw ?? {},
    unavailable: drawing?.unavailable ?? {},
    cursorFx: cursorFx ?? {},
    terminal,
    transport,
    intoTerminal,
    echoBurst,
    echoSpaced,
    flood: floodDrawing,
    echoShell,
    hydrate,
    problems,
  };
}
