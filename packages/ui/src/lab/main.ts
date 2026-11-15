/**
 * The renderer lab: a development page that draws a fixed set of frames with every
 * renderer tier side by side.
 *
 * It answers the questions a renderer change raises. Does each tier draw the frame at
 * all? Do the tiers draw the same picture, pixel for pixel? How long does a full redraw
 * take? Does a pane survive losing its tier? The numbers are also left on `window.__lab`
 * so a script driving the page can read them.
 *
 * The page is served by the dev server only and is not part of the built app.
 */

import {
  RenderHost,
  rgba,
  TIER_ORDER,
  type CellRenderer,
  type RenderFrame,
  type TerminalTheme,
  type Tier,
} from "@lume/renderer";
import { FIXED_FRAMES, fullRedrawFrame } from "@lume/renderer/testing";
import { Vt } from "@lume/vt";
import wasmUrl from "@lume/vt/ghostty-vt.wasm?url";

import fontUrl from "../../../../tests/fonts/JetBrainsMono-Regular.ttf?url";
import { checkFrameContract } from "../frame-contract.ts";
import "./lab.css";
import { fullRedrawOutput, terminalFrames, type TerminalFrame } from "./workloads.ts";

/** A family name no installed font has, so the bundled file is the one measured. */
const FONT_FAMILY = "Lume Lab Mono";

/** Frames drawn per tier when timing. */
const TIMING_FRAMES = 120;

/** Colour channels may differ by this much between tiers and still count as equal. */
const COMPARE_TOLERANCE = 2;

const theme: TerminalTheme = {
  // The system fonts follow for what the test font lacks: CJK and emoji.
  font: { family: `"${FONT_FAMILY}", monospace`, sizePx: 14 },
  cursor: rgba(0xf0, 0xf0, 0xf0),
  selectionBackground: rgba(0x2f, 0x4f, 0x8f),
  padding: 4,
};

interface Pane {
  /** The tier this pane is restricted to, or `"auto"` for the normal order. */
  kind: Tier | "auto";
  host: RenderHost;
  renderer: CellRenderer | undefined;
  root: HTMLElement;
  surface: HTMLElement;
  note: HTMLElement;
  error: string | undefined;
}

interface Timing {
  p50: number;
  p95: number;
}

interface Results {
  ready: boolean;
  frame: string;
  devicePixelRatio: number;
  tiers: Record<string, string>;
  /** Pixels that differ from the Canvas 2D picture: exactly, and beyond the tolerance. */
  differences: Record<string, { exact: number; beyondTolerance: number; pixels: number }>;
  timings: Record<string, { draw: Timing; drawAndFinish: Timing }>;
  terminal: { write: Timing; read: Timing } | undefined;
  log: string[];
}

const results: Results = {
  ready: false,
  frame: "",
  devicePixelRatio: window.devicePixelRatio,
  tiers: {},
  differences: {},
  timings: {},
  terminal: undefined,
  log: [],
};

declare global {
  interface Window {
    /** The lab's results and controls, for a script driving the page. */
    __lab: {
      results: Results;
      show(name: string): void;
      time(): Promise<void>;
      lose(): string;
      /** What a pane last drew, as a PNG. */
      capture(kind: string): Promise<Blob | undefined>;
      frames: string[];
    };
  }
}

function element<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const made = document.createElement(tag);
  if (className) made.className = className;
  if (text !== undefined) made.textContent = text;
  return made;
}

function log(message: string): void {
  results.log.push(message);
  console.info(`[lab] ${message}`);
}

function percentile(sorted: number[], p: number): number {
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))] ?? 0;
}

function timing(samples: number[]): Timing {
  const sorted = [...samples].sort((a, b) => a - b);
  const round = (ms: number) => Math.round(ms * 1000) / 1000;
  return { p50: round(percentile(sorted, 0.5)), p95: round(percentile(sorted, 0.95)) };
}

/**
 * The next animation frame. A hidden page gets none, so after a short wait it goes on
 * anyway: the lab must also run where nobody is looking at it.
 */
function nextFrame(): Promise<void> {
  return new Promise((resolve) => {
    const fallback = setTimeout(resolve, 50);
    requestAnimationFrame(() => {
      clearTimeout(fallback);
      resolve();
    });
  });
}

async function loadFont(): Promise<void> {
  // Loaded from its bytes: fetching needs no more than the page's own origin.
  const bytes = await (await fetch(fontUrl)).arrayBuffer();
  const face = new FontFace(FONT_FAMILY, bytes);
  await face.load();
  document.fonts.add(face);
}

/** The canvas a renderer put into its pane. */
function canvasOf(pane: Pane): HTMLCanvasElement | undefined {
  return pane.surface.querySelector("canvas") ?? undefined;
}

/** A copy of what a canvas shows right now. Call it in the task that drew the canvas. */
function readPixels(canvas: HTMLCanvasElement): ImageData {
  const copy = element("canvas");
  copy.width = canvas.width;
  copy.height = canvas.height;
  const context = copy.getContext("2d", { willReadFrequently: true });
  if (!context) throw new Error("no 2D context for reading pixels back");
  context.drawImage(canvas, 0, 0);
  return context.getImageData(0, 0, copy.width, copy.height);
}

function compare(
  a: ImageData,
  b: ImageData,
): { exact: number; beyondTolerance: number; pixels: number } {
  if (a.width !== b.width || a.height !== b.height) {
    return { exact: -1, beyondTolerance: -1, pixels: a.width * a.height };
  }
  let exact = 0;
  let beyond = 0;
  for (let i = 0; i < a.data.length; i += 4) {
    const delta = Math.max(
      Math.abs(a.data[i]! - b.data[i]!),
      Math.abs(a.data[i + 1]! - b.data[i + 1]!),
      Math.abs(a.data[i + 2]! - b.data[i + 2]!),
    );
    if (delta > 0) exact++;
    if (delta > COMPARE_TOLERANCE) beyond++;
  }
  return { exact, beyondTolerance: beyond, pixels: a.width * a.height };
}

/** Size a pane's renderer so that the frame's grid fits exactly. */
function fit(pane: Pane, frame: RenderFrame): void {
  const renderer = pane.renderer;
  if (!renderer) return;
  const dpr = window.devicePixelRatio;
  // The metrics depend on the pixel ratio, so it is set before they are read.
  renderer.resize({ width: 1, height: 1 }, dpr);
  const m = renderer.metrics;
  renderer.resize(
    {
      width: (frame.cols * m.cellWidth + 2 * m.padding) / dpr,
      height: (frame.rows * m.cellHeight + 2 * m.padding) / dpr,
    },
    dpr,
  );
}

async function main(): Promise<void> {
  const root = document.getElementById("lab");
  if (!root) throw new Error("lab.html has no #lab element");
  checkFrameContract();
  await loadFont();
  const vt = await Vt.instantiate(fetch(wasmUrl));
  const fromTerminal = terminalFrames(vt);

  root.append(element("h1", undefined, "Lume renderer lab"));
  const controls = element("div", "controls");
  const picker = element("select");
  const names = [...FIXED_FRAMES.map((fixed) => fixed.name), ...Object.keys(fromTerminal)];
  for (const name of names) picker.append(new Option(name, name));
  const timeButton = element("button", undefined, "Time a full redraw");
  const loseButton = element("button", undefined, "Lose the tier of the last pane");
  const status = element("span", "status");
  controls.append(picker, timeButton, loseButton, status);
  root.append(controls);

  const panesRoot = element("div", "panes");
  root.append(panesRoot);
  const table = element("table");
  root.append(table);

  // One pane per tier, each held to that tier, and one that takes the normal order and
  // is the one to lose its tier.
  const panes: Pane[] = [];
  for (const kind of [...TIER_ORDER, "auto"] as const) {
    const host = await RenderHost.create(kind === "auto" ? {} : { tiers: [kind] });
    const paneRoot = element("section", "pane");
    const title = element("h2", undefined, kind === "auto" ? "automatic (falls back)" : kind);
    const note = element("div", "note");
    const surface = element("div", "surface");
    paneRoot.append(title, note, surface);
    panesRoot.append(paneRoot);
    const pane: Pane = {
      kind,
      host,
      renderer: undefined,
      root: paneRoot,
      surface,
      note,
      error: undefined,
    };
    try {
      pane.renderer = host.createRenderer(surface, theme, {
        onTierChange: (tier, reason) => {
          log(`${kind}: now ${tier ?? "nothing"} (${reason})`);
          show(picker.value);
        },
        onInvalidate: () => void nextFrame().then(() => show(picker.value)),
      });
      results.tiers[kind] = pane.renderer.tier;
    } catch (error) {
      pane.error = error instanceof Error ? error.message : String(error);
      paneRoot.classList.add("unavailable");
      results.tiers[kind] = `unavailable: ${pane.error}`;
    }
    panes.push(pane);
  }

  let current: TerminalFrame | undefined;
  /** What each pane drew for the frame on show, read back in the task that drew it. */
  const pictures = new Map<Pane, ImageData>();

  /** Draw the named frame on every pane and compare the pictures. */
  function show(name: string): void {
    current?.free();
    current = undefined;
    const fixed = FIXED_FRAMES.find((candidate) => candidate.name === name);
    let frame: RenderFrame;
    if (fixed) {
      frame = fixed.build();
    } else {
      current = fromTerminal[name]?.();
      if (!current) throw new Error(`there is no frame called ${name}`);
      frame = current.frame;
    }
    results.frame = name;
    results.differences = {};
    // Keep the menu in step when a script, not the menu, chose the frame.
    picker.value = name;

    pictures.clear();
    for (const pane of panes) {
      if (!pane.renderer) {
        pane.note.textContent = pane.error ?? "unavailable";
        continue;
      }
      fit(pane, frame);
      // Every pane draws the whole frame, whatever the last one left flagged.
      frame.dirty = 2;
      frame.dirtyRows.fill(1);
      pane.renderer.draw(frame);
      const canvas = canvasOf(pane);
      if (canvas) pictures.set(pane, readPixels(canvas));
      results.tiers[pane.kind] = pane.renderer.tier;
    }

    const reference = panes.find((pane) => pane.kind === "canvas2d");
    const base = reference && pictures.get(reference);
    for (const pane of panes) {
      const picture = pictures.get(pane);
      if (!pane.renderer || !picture) continue;
      const m = pane.renderer.metrics;
      let note = `tier ${pane.renderer.tier}, cell ${m.cellWidth}×${m.cellHeight} px, surface ${picture.width}×${picture.height} px`;
      if (base && pane !== reference) {
        const difference = compare(picture, base);
        results.differences[pane.kind] = difference;
        note += `\nagainst canvas2d: ${difference.exact} of ${difference.pixels} pixels differ, ${difference.beyondTolerance} by more than ${COMPARE_TOLERANCE}`;
      }
      pane.note.textContent = note;
    }
    status.textContent = `showing ${name}`;
  }

  /** Time alternating full redraws of a 200 by 60 screen on every pane. */
  async function time(): Promise<void> {
    status.textContent = "timing…";
    await nextFrame();
    const frames = [fullRedrawFrame(0), fullRedrawFrame(1)];
    const probe = element("canvas");
    probe.width = 1;
    probe.height = 1;
    const probeContext = probe.getContext("2d", { willReadFrequently: true });

    for (const pane of panes) {
      const renderer = pane.renderer;
      if (!renderer || pane.kind === "auto") continue;
      fit(pane, frames[0]!);
      const draw: number[] = [];
      const finish: number[] = [];
      // A few frames first, so the atlas holds every glyph the workload uses.
      for (let i = -8; i < TIMING_FRAMES; i++) {
        const frame = frames[i & 1]!;
        frame.dirty = 2;
        frame.dirtyRows.fill(1);
        const start = performance.now();
        renderer.draw(frame);
        const drawn = performance.now();
        // Reading a pixel back makes the browser wait for the GPU to finish the
        // frame, so this time includes the drawing itself and not only queueing it.
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
      results.timings[pane.kind] = { draw: timing(draw), drawAndFinish: timing(finish) };
      await nextFrame();
    }

    // The terminal's side of the same workload: taking the output in, and reading the
    // screen back out as a frame.
    const terminal = fromTerminal["vt: full-200x60"]!();
    const outputs = [fullRedrawOutput(0), fullRedrawOutput(1)];
    const write: number[] = [];
    const read: number[] = [];
    for (let i = 0; i < TIMING_FRAMES; i++) {
      const start = performance.now();
      terminal.write(outputs[i & 1]!);
      const written = performance.now();
      terminal.update();
      const updated = performance.now();
      terminal.markDrawn();
      write.push(written - start);
      read.push(updated - written);
    }
    terminal.free();
    results.terminal = { write: timing(write), read: timing(read) };

    renderTimings();
    show(picker.value);
    status.textContent = "timed";
  }

  function renderTimings(): void {
    table.replaceChildren();
    const head = element("tr");
    for (const title of ["200×60 full redraw", "p50 ms", "p95 ms"]) {
      head.append(element("th", undefined, title));
    }
    table.append(head);
    const row = (label: string, value: Timing) => {
      const tr = element("tr");
      tr.append(
        element("td", undefined, label),
        element("td", undefined, value.p50.toFixed(3)),
        element("td", undefined, value.p95.toFixed(3)),
      );
      table.append(tr);
    };
    for (const [tier, value] of Object.entries(results.timings)) {
      row(`${tier}: build and submit`, value.draw);
      row(`${tier}: until the GPU has finished`, value.drawAndFinish);
    }
    if (results.terminal) {
      row("terminal: take the output in", results.terminal.write);
      row("terminal: read the screen as a frame", results.terminal.read);
    }
  }

  /** Take away the tier of the automatic pane, as a driver reset would. */
  function lose(): string {
    const pane = panes.find((candidate) => candidate.kind === "auto");
    const canvas = pane && canvasOf(pane);
    if (!pane?.renderer || !canvas) return "the automatic pane has no renderer";
    const tier = pane.renderer.tier;
    if (tier === "webgl2") {
      const lost = canvas.getContext("webgl2")?.getExtension("WEBGL_lose_context");
      if (!lost) return "this WebGL2 context cannot be lost on request";
      lost.loseContext();
    } else if (tier === "webgpu") {
      const context = canvas.getContext("webgpu");
      if (!(context instanceof GPUCanvasContext)) return "no WebGPU context on the canvas";
      const device = context.getConfiguration()?.device;
      if (!device) return "this engine does not say which device a canvas uses";
      device.destroy();
    } else {
      return "Canvas 2D is the last tier; there is nothing to fall back to";
    }
    return `asked ${tier} to go away`;
  }

  function capture(kind: string): Promise<Blob | undefined> {
    const pane = panes.find((candidate) => candidate.kind === kind);
    const picture = pane && pictures.get(pane);
    if (!picture) return Promise.resolve(undefined);
    const copy = element("canvas");
    copy.width = picture.width;
    copy.height = picture.height;
    copy.getContext("2d")?.putImageData(picture, 0, 0);
    return new Promise((resolve) => copy.toBlob((blob) => resolve(blob ?? undefined), "image/png"));
  }

  picker.addEventListener("change", () => show(picker.value));
  timeButton.addEventListener("click", () => void time());
  loseButton.addEventListener("click", () => {
    status.textContent = lose();
  });

  window.__lab = { results, show, time, lose, capture, frames: names };
  show(names[0]!);
  results.ready = true;
}

main().catch((error: unknown) => {
  results.log.push(
    `failed: ${error instanceof Error ? (error.stack ?? error.message) : String(error)}`,
  );
  window.__lab = {
    results,
    show: () => undefined,
    time: () => Promise.resolve(),
    lose: () => "the lab failed to start",
    capture: () => Promise.resolve(undefined),
    frames: [],
  };
  console.error(error);
  document.body.append(element("pre", undefined, String(error)));
});
