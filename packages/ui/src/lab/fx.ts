/**
 * The cursor's effects in the lab: every effect played on demand with WebGL2 and with
 * Canvas 2D side by side, over a grid of cells the size of a terminal's, for tuning
 * by eye. The real effects, drawn by a frame loop of their own.
 */

import {
  Canvas2dFx,
  CursorEffects,
  FrameLoop,
  rgba,
  WebGl2Fx,
  type FxRect,
  type FxSurface,
  type PulseKind,
  type TrailKind,
} from "@lume/renderer";

/** An effect the strip can play. */
export type LabEffect = Exclude<TrailKind, "none"> | Exclude<PulseKind, "none">;

const TRAILS: readonly LabEffect[] = ["warp", "sweep", "comet"];
const PULSES: readonly LabEffect[] = ["burst", "ripple", "box-burst", "box-ripple"];

/** The size of a stage, in CSS pixels, and of its cells. */
const STAGE = { width: 560, height: 260 };
const CELL = { width: 9, height: 18 };
const COLOR = rgba(122, 167, 255);

/** The places a trail goes between, one after another, in cells. */
const STOPS: readonly (readonly [number, number])[] = [
  [3, 2],
  [55, 2],
  [10, 12],
  [40, 8],
];

function cellRect(col: number, row: number): FxRect {
  return { x: col * CELL.width, y: row * CELL.height, ...CELL };
}

function surfaceIn(parent: HTMLElement): FxSurface<HTMLCanvasElement> {
  return {
    createCanvas() {
      const canvas = document.createElement("canvas");
      canvas.className = "fx-canvas";
      return canvas;
    },
    show: (canvas) => parent.append(canvas),
    remove: (canvas) => canvas.remove(),
    setCssSize(canvas, size) {
      canvas.style.width = `${size.width}px`;
      canvas.style.height = `${size.height}px`;
    },
    setVisible(canvas, visible) {
      canvas.style.visibility = visible ? "visible" : "hidden";
    },
  };
}

/** Builds the strip in `parent`, and returns what plays an effect on one or both. */
export function mountFxStrip(
  parent: HTMLElement,
): (effect: LabEffect, backend?: "webgl2" | "canvas2d") => string {
  const section = document.createElement("section");
  section.className = "fx";
  const title = document.createElement("h2");
  title.textContent = "Cursor effects";
  const buttons = document.createElement("div");
  buttons.className = "controls";
  const stages = document.createElement("div");
  stages.className = "fx-stages";
  section.append(title, buttons, stages);
  parent.append(section);

  const loop = new FrameLoop();
  const players = (["webgl2", "canvas2d"] as const).map((kind) => {
    const stage = document.createElement("div");
    stage.className = "fx-stage";
    stage.style.width = `${STAGE.width}px`;
    stage.style.height = `${STAGE.height}px`;
    const label = document.createElement("div");
    label.className = "fx-label";
    stage.append(label);
    stages.append(stage);
    const effects = new CursorEffects<HTMLCanvasElement>(
      surfaceIn(stage),
      {
        webgl2: kind === "webgl2" ? (canvas) => new WebGl2Fx(canvas) : undefined,
        canvas2d: (canvas) => new Canvas2dFx(canvas),
      },
      () => loop.requestFrame(),
    );
    loop.add(effects, { after: true });
    effects.resize(STAGE, window.devicePixelRatio);
    return { kind, effects, label, stop: 0 };
  });

  const play = (effect: LabEffect, only?: "webgl2" | "canvas2d"): string => {
    const trail = TRAILS.includes(effect);
    const now = performance.now();
    const drawnWith: string[] = [];
    for (const player of players) {
      if (only !== undefined && player.kind !== only) continue;
      player.effects.configure(
        trail
          ? { trail: effect as Exclude<TrailKind, "none">, pulse: "none" }
          : { trail: "none", pulse: effect as Exclude<PulseKind, "none"> },
      );
      const [fromCol, fromRow] = STOPS[player.stop % STOPS.length]!;
      player.stop++;
      const [toCol, toRow] = STOPS[player.stop % STOPS.length]!;
      const to = cellRect(toCol, toRow);
      if (trail) player.effects.move(cellRect(fromCol, fromRow), to, COLOR, now);
      else player.effects.pulse(to, COLOR, now);
      player.label.textContent = `${player.kind} → ${player.effects.backend ?? "nothing"}`;
      drawnWith.push(player.effects.backend ?? "nothing");
    }
    return drawnWith.join(", ");
  };

  for (const effect of [...TRAILS, ...PULSES]) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = effect;
    button.addEventListener("click", () => void play(effect));
    buttons.append(button);
  }
  return play;
}
