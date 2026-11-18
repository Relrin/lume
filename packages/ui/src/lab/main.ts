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

import type { RenderFrame } from "@lume/renderer";
import { FIXED_FRAMES, fullRedrawFrame } from "@lume/renderer/testing";
import { Vt } from "@lume/vt";
import wasmUrl from "@lume/vt/ghostty-vt.wasm?url";

import { checkFrameContract } from "../frame-contract.ts";
import {
  canvasOf,
  createPane,
  drawWhole,
  fit,
  loadTestFont,
  TIERS,
  type TierPane,
} from "../harness/panes.ts";
import { compare, readPixels, toPng, type Difference } from "../harness/pixels.ts";
import { nextFrame, timing, type Timing } from "../harness/timing.ts";
import { COMPARE_TOLERANCE, REDRAW_FRAMES, REDRAW_WARMUP_FRAMES } from "../harness/tuning.ts";
import { fullRedrawOutput, terminalFrames, type TerminalFrame } from "../harness/workloads.ts";
import { mountFxStrip, type LabEffect } from "./fx.ts";
import "./lab.css";

/** A pane of the lab: a renderer and the elements around it. */
interface Pane extends TierPane {
  readonly root: HTMLElement;
  readonly note: HTMLElement;
}

interface Results {
  ready: boolean;
  frame: string;
  devicePixelRatio: number;
  tiers: Record<string, string>;
  /** How each pane's picture differs from the Canvas 2D one. */
  differences: Record<string, Difference>;
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
      /** Plays a cursor effect, with one backend or both; says what drew it. */
      fx(effect: LabEffect, backend?: "webgl2" | "canvas2d"): string;
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

async function main(): Promise<void> {
  const root = document.getElementById("lab");
  if (!root) throw new Error("lab.html has no #lab element");
  checkFrameContract();
  await loadTestFont();
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
  for (const kind of [...TIERS, "auto"] as const) {
    const paneRoot = element("section", "pane");
    const title = element("h2", undefined, kind === "auto" ? "automatic (falls back)" : kind);
    const note = element("div", "note");
    const surface = element("div", "surface");
    paneRoot.append(title, note, surface);
    panesRoot.append(paneRoot);
    const made = await createPane(kind, surface, {
      onTierChange: (tier, reason) => {
        log(`${kind}: now ${tier ?? "nothing"} (${reason})`);
        show(picker.value);
      },
      onInvalidate: () => void nextFrame().then(() => show(picker.value)),
    });
    if (made.renderer) {
      results.tiers[kind] = made.renderer.tier;
    } else {
      paneRoot.classList.add("unavailable");
      results.tiers[kind] = `unavailable: ${made.error}`;
    }
    panes.push({ ...made, root: paneRoot, note });
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
      fit(pane, frame, window.devicePixelRatio);
      // Every pane draws the whole frame, whatever the last one left flagged.
      drawWhole(pane, frame);
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
        const difference = compare(picture, base, COMPARE_TOLERANCE);
        results.differences[pane.kind] = difference;
        note += difference.sameSize
          ? `\nagainst canvas2d: ${difference.exact} of ${difference.pixels} pixels differ, ${difference.beyondTolerance} by more than ${COMPARE_TOLERANCE}`
          : "\nagainst canvas2d: the pictures are not the same size";
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
      fit(pane, frames[0]!, window.devicePixelRatio);
      const draw: number[] = [];
      const finish: number[] = [];
      // A few frames first, so the atlas holds every glyph the workload uses.
      for (let i = -REDRAW_WARMUP_FRAMES; i < REDRAW_FRAMES; i++) {
        const frame = frames[i & 1]!;
        const start = performance.now();
        drawWhole(pane, frame);
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
    for (let i = 0; i < REDRAW_FRAMES; i++) {
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
    return toPng(picture).then((png) => new Blob([png as BlobPart], { type: "image/png" }));
  }

  picker.addEventListener("change", () => show(picker.value));
  timeButton.addEventListener("click", () => void time());
  loseButton.addEventListener("click", () => {
    status.textContent = lose();
  });

  const fx = mountFxStrip(document.getElementById("lab") ?? document.body);
  window.__lab = { results, show, time, lose, capture, fx, frames: names };
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
    fx: () => "the lab failed to start",
    frames: [],
  };
  console.error(error);
  document.body.append(element("pre", undefined, String(error)));
});
