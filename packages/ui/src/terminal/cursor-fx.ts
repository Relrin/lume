/**
 * Where the cursor's effects are told what the cursor does.
 *
 * Each terminal view says, after it draws, where its cursor is and whether it has the
 * keyboard. Only the cursor of the pane with the keyboard plays effects: a jump of it
 * leaves a trail, a change of its shape gives off a pulse, and the keyboard moving to
 * another pane does both, from the old pane's cursor to the new one's. A view says when
 * its cursor moved only because the grid did (a resize, a font change), and that plays
 * nothing; nor does the first cursor seen after the panes on show change wholesale, as
 * when another tab is shown.
 *
 * The effects stop while the system asks for less motion.
 *
 * A trail or a pulse may be a plugin's, drawn by its shader. One whose plugin is not
 * there, or whose shader could not be built, draws nothing.
 *
 * A motion, Lume's own glide or a plugin's, draws the cursor itself on its way from one
 * place to the next, with every step it takes, typing included: the pane it ends in
 * leaves its own cursor out until the motion is over. It plays beside a trail, not in
 * place of one.
 */

import type {
  CursorEffectSettings,
  CustomEffect,
  FxRect,
  MotionKind,
  PulseKind,
  TrailKind,
} from "@lume/renderer";

import type { ConfigView, CursorEffectDef, UiContributions } from "../core/types.ts";
import { assets } from "../plugins/assets.ts";

/** The window's effects, as far as this needs them. */
export interface EffectsLike {
  configure(settings: CursorEffectSettings): void;
  resize(size: { width: number; height: number }, dpr: number): void;
  move(from: FxRect, to: FxRect, color: number, now: number): boolean;
  /** The cursor moved, from and to what it covers: a plugin's motion draws it on its way. */
  glide?(from: FxRect, to: FxRect, color: number, now: number): boolean;
  pulse(at: FxRect, color: number, now: number): boolean;
  /**
   * Until when the motion playing draws the cursor in place of its pane, in the time of
   * the frames: `-Infinity`, or left out, while none does.
   */
  readonly cursorDrawnUntil?: number;
  cancel(): void;
  dispose(): void;
  /** How many trails and pulses have started. */
  readonly played?: { readonly trails: number; readonly pulses: number };
  /** Told when a plugin's effect cannot be drawn. */
  onProblem?(listener: (id: string, message: string) => void): void;
}

/** A view whose cursor the effects follow. */
export interface CursorSource {
  /**
   * Where a cell of its screen is now, in CSS pixels of the window, or nothing when the
   * view is not on show. Asked only when an effect plays: it reads the page's layout.
   */
  cellRect(col: number, row: number): FxRect | undefined;
  /**
   * What the cursor covers in a cell, drawn in a shape (a `CursorShape`) and across two
   * cells for a wide character, in CSS pixels of the window: for a motion, which draws
   * the cursor itself. Left out, the cell.
   */
  cursorRect?(col: number, row: number, shape: number, wide: boolean): FxRect | undefined;
  /** The effects stopped drawing its cursor sooner than they said they would. */
  cursorReleased?(): void;
}

/** What a view says of its cursor after drawing. */
export interface CursorReport {
  /** The cursor's cell and shape, as the frame has them. */
  readonly col: number;
  readonly row: number;
  readonly shape: number;
  /** Whether it covers a wide character's two cells. Left out, it does not. */
  readonly wide?: boolean;
  /** Whether the cursor is drawn at all: shown, and in the part of the screen on show. */
  readonly visible: boolean;
  /** Whether the view has the keyboard. */
  readonly focused: boolean;
  /** Whether the cursor moved only because the grid changed under it. */
  readonly reflowed: boolean;
  /** The cursor's colour, packed. */
  readonly color: number;
}

/** What the views are given: whether to say anything, and where to say it. */
export interface CursorWatch {
  /** Whether any effect is on. A view says nothing while none is. */
  readonly watching: boolean;
  /**
   * Told before the view draws. Gives until when the effects draw the view's cursor in
   * its place, in the time of the frames: the view leaves its own out until then. While
   * they do not, `-Infinity`.
   */
  report(source: CursorSource, report: CursorReport, now: number): number;
  /** Whether the effects still draw the view's cursor. */
  holds(source: CursorSource): boolean;
  /** A view is no longer on show. */
  forget(source: CursorSource): void;
}

/** The settings, as the configuration has them. */
export interface CursorFxSettings {
  readonly trail: TrailKind;
  readonly pulse: PulseKind;
  /** How the cursor itself moves to its new place. Left out, at once. */
  readonly motion?: MotionKind | undefined;
  /** How long each effect lasts against its own length. Left out, 1. */
  readonly length?: number | undefined;
  /** A plugin's shader for the trail, in place of the built-in one. */
  readonly customTrail?: CustomEffect | undefined;
  /** A plugin's shader for the pulse, in place of the built-in one. */
  readonly customPulse?: CustomEffect | undefined;
  /** A plugin's shader that draws the cursor on its way, in place of the built-in one. */
  readonly customMotion?: CustomEffect | undefined;
}

interface Last {
  readonly source: CursorSource;
  readonly col: number;
  readonly row: number;
  readonly shape: number;
  readonly wide: boolean;
}

const OFF: CursorFxSettings = { trail: "none", pulse: "none" };

const TRAILS: readonly string[] = ["none", "warp", "sweep", "comet"];
const PULSES: readonly string[] = ["none", "burst", "ripple", "box-burst", "box-ripple"];
const MOTIONS: readonly string[] = ["none", "glide", "glide-slow"];

/**
 * A plugin's effect of a kind, by the id the settings name it with, once its shader
 * has been fetched: none until then.
 */
function customOf(
  shown: UiContributions | null | undefined,
  id: string,
  kind: CursorEffectDef["kind"],
): CustomEffect | undefined {
  const found = shown?.cursor_effects.find((effect) => effect.id === id && effect.kind === kind);
  const source = found ? assets.peek(found.hash) : undefined;
  return found && source !== undefined
    ? { id: found.id, source, duration: found.duration, reach: found.reach }
    : undefined;
}

/**
 * The hashes of the shaders a configuration's cursor effects need: those of the
 * plugins' effects it names, and of no other.
 */
export function cursorFxShaders(view: ConfigView | null, shown?: UiContributions | null): string[] {
  const terminal = view?.config.terminal;
  const named = [
    ["trail", terminal?.["cursor-trail"] ?? "none"],
    ["motion", terminal?.["cursor-motion"] ?? "none"],
    ["pulse", terminal?.["cursor-pulse"] ?? "none"],
  ] as const;
  const hashes: string[] = [];
  for (const [kind, id] of named) {
    const found = shown?.cursor_effects.find((effect) => effect.id === id && effect.kind === kind);
    if (found) hashes.push(found.hash);
  }
  return hashes;
}

/**
 * The effects a configuration asks for, with the plugins' effects it names. A core that
 * does not say asks for none.
 */
export function cursorFxSettings(
  view: ConfigView | null,
  shown?: UiContributions | null,
): CursorFxSettings {
  const terminal = view?.config.terminal;
  const length = terminal?.["cursor-effect-length"];
  const trail = terminal?.["cursor-trail"] ?? "none";
  const pulse = terminal?.["cursor-pulse"] ?? "none";
  const customTrail = TRAILS.includes(trail) ? undefined : customOf(shown, trail, "trail");
  const customPulse = PULSES.includes(pulse) ? undefined : customOf(shown, pulse, "pulse");
  const motion = terminal?.["cursor-motion"] ?? "none";
  const customMotion = MOTIONS.includes(motion) ? undefined : customOf(shown, motion, "motion");
  return {
    trail: TRAILS.includes(trail) ? (trail as TrailKind) : "none",
    pulse: PULSES.includes(pulse) ? (pulse as PulseKind) : "none",
    ...(motion !== "none" && MOTIONS.includes(motion) ? { motion: motion as MotionKind } : {}),
    ...(typeof length === "number" ? { length } : {}),
    ...(customTrail ? { customTrail } : {}),
    ...(customPulse ? { customPulse } : {}),
    ...(customMotion ? { customMotion } : {}),
  };
}

/** Whether settings ask for any effect at all. */
function anyEffect(settings: CursorFxSettings): boolean {
  return (
    settings.trail !== "none" ||
    settings.pulse !== "none" ||
    (settings.motion ?? "none") !== "none" ||
    settings.customTrail !== undefined ||
    settings.customPulse !== undefined ||
    settings.customMotion !== undefined
  );
}

/** The window's cursor effects and what they follow. */
export class CursorFx implements CursorWatch {
  readonly #make: (container: HTMLElement) => EffectsLike;
  readonly #reducedMotion: { readonly matches: boolean } | undefined;
  #effects: EffectsLike | undefined;
  #settings: CursorFxSettings = OFF;
  #size = { width: 0, height: 0 };
  #dpr = 1;
  #last: Last | undefined;
  /** The view whose cursor the effects draw, while they may. */
  #holder: CursorSource | undefined;
  #problem: ((id: string, message: string) => void) | undefined;

  /**
   * `make` makes the effects inside the element the page gives them, and
   * `reducedMotion` says whether the system asks for less motion.
   */
  constructor(
    make: (container: HTMLElement) => EffectsLike,
    reducedMotion?: { readonly matches: boolean },
  ) {
    this.#make = make;
    this.#reducedMotion = reducedMotion;
  }

  get watching(): boolean {
    return this.#effects !== undefined && this.#wanted() !== OFF;
  }

  /** How many trails and pulses have started in this window, for measurements. */
  played(): { readonly trails: number; readonly pulses: number } {
    return this.#effects?.played ?? { trails: 0, pulses: 0 };
  }

  /** Told when a plugin's effect cannot be drawn: its id and why. Gives how to stop. */
  onProblem(listener: (id: string, message: string) => void): () => void {
    this.#problem = listener;
    this.#effects?.onProblem?.(listener);
    return () => {
      if (this.#problem !== listener) return;
      this.#problem = undefined;
      this.#effects?.onProblem?.(() => undefined);
    };
  }

  /** The element the effects are drawn in, from the page. */
  attach(container: HTMLElement): void {
    this.detach();
    this.#effects = this.#make(container);
    if (this.#problem) this.#effects.onProblem?.(this.#problem);
    this.#effects.resize(this.#size, this.#dpr);
    this.#effects.configure(this.#wanted());
  }

  detach(): void {
    this.#effects?.dispose();
    this.#effects = undefined;
    this.#last = undefined;
    this.#release();
  }

  /** The settings changed, or the system's wish for less motion did. */
  configure(settings: CursorFxSettings = this.#settings): void {
    this.#settings = settings;
    const wanted = this.#wanted();
    if (wanted === OFF) {
      this.#effects?.cancel();
      this.#last = undefined;
    }
    this.#effects?.configure(wanted);
    // Another trail, or none: what drew the cursor may have stopped.
    if (!this.#drawing()) this.#release();
  }

  /** The window's size in CSS pixels, and its device pixel ratio. */
  resize(size: { width: number; height: number }, dpr: number): void {
    this.#size = size;
    this.#dpr = dpr;
    this.#effects?.resize(size, dpr);
  }

  report(source: CursorSource, report: CursorReport, now: number): number {
    this.#play(source, report, now);
    return this.#heldFor(source);
  }

  holds(source: CursorSource): boolean {
    return this.#holder === source && this.#drawing();
  }

  forget(source: CursorSource): void {
    if (this.#last?.source === source) this.#last = undefined;
    if (this.#holder === source) this.#holder = undefined;
  }

  #play(source: CursorSource, report: CursorReport, now: number): void {
    const effects = this.#effects;
    if (!effects || !this.watching) return;
    // A cursor without the keyboard plays nothing, and is not where the keyboard goes
    // from: the pane that last had it is. Its pane draws it itself again.
    if (!report.focused || !report.visible) {
      if (this.#holder === source) this.#holder = undefined;
      return;
    }
    const last = this.#last;
    const wide = report.wide === true;
    this.#last = { source, col: report.col, row: report.row, shape: report.shape, wide };
    if (!last) return;
    if (last.source !== source) {
      const to = source.cellRect(report.col, report.row);
      if (!to) return;
      const from = last.source.cellRect(last.col, last.row);
      if (from) effects.move(from, to, report.color, now);
      this.#glide(source, last, report, now);
      effects.pulse(to, report.color, now);
      return;
    }
    if (report.reflowed) return;
    const moved = last.col !== report.col || last.row !== report.row;
    const reshaped = last.shape !== report.shape;
    if (!moved && !reshaped) return;
    if (moved) {
      const from = source.cellRect(last.col, last.row);
      const to = source.cellRect(report.col, report.row);
      if (from && to) effects.move(from, to, report.color, now);
      this.#glide(source, last, report, now);
    }
    if (reshaped) {
      const to = source.cellRect(report.col, report.row);
      if (to) effects.pulse(to, report.color, now);
    }
  }

  /**
   * Starts the motion from what the cursor covered to what it covers now, and notes
   * whose cursor it draws.
   */
  #glide(source: CursorSource, last: Last, report: CursorReport, now: number): void {
    const effects = this.#effects;
    const { motion = "none", customMotion } = this.#settings;
    if (!effects?.glide || (motion === "none" && customMotion === undefined)) return;
    const from = this.#covered(last.source, last.col, last.row, last.shape, last.wide);
    const to = this.#covered(source, report.col, report.row, report.shape, report.wide === true);
    if (!from || !to || !effects.glide(from, to, report.color, now)) return;
    this.#holder = source;
  }

  /** What the cursor covers in a cell, or the cell where the view cannot say. */
  #covered(
    source: CursorSource,
    col: number,
    row: number,
    shape: number,
    wide: boolean,
  ): FxRect | undefined {
    return source.cursorRect?.(col, row, shape, wide) ?? source.cellRect(col, row);
  }

  /** Whether the motion playing draws the cursor. */
  #drawing(): boolean {
    return (this.#effects?.cursorDrawnUntil ?? -Infinity) > -Infinity;
  }

  #heldFor(source: CursorSource): number {
    if (this.#holder !== source) return -Infinity;
    return this.#effects?.cursorDrawnUntil ?? -Infinity;
  }

  /** Gives the cursor back to the view whose cursor the effects drew. */
  #release(): void {
    const holder = this.#holder;
    this.#holder = undefined;
    holder?.cursorReleased?.();
  }

  #wanted(): CursorFxSettings {
    if (this.#reducedMotion?.matches === true) return OFF;
    if (!anyEffect(this.#settings)) return OFF;
    return this.#settings;
  }
}
