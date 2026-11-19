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
 */

import type {
  CursorEffectSettings,
  CustomEffect,
  FxRect,
  PulseKind,
  TrailKind,
} from "@lume/renderer";

import type { ConfigView, UiContributions } from "../core/types.ts";

/** The window's effects, as far as this needs them. */
export interface EffectsLike {
  configure(settings: CursorEffectSettings): void;
  resize(size: { width: number; height: number }, dpr: number): void;
  move(from: FxRect, to: FxRect, color: number, now: number): boolean;
  pulse(at: FxRect, color: number, now: number): boolean;
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
}

/** What a view says of its cursor after drawing. */
export interface CursorReport {
  /** The cursor's cell and shape, as the frame has them. */
  readonly col: number;
  readonly row: number;
  readonly shape: number;
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
  report(source: CursorSource, report: CursorReport, now: number): void;
  /** A view is no longer on show. */
  forget(source: CursorSource): void;
}

/** The settings, as the configuration has them. */
export interface CursorFxSettings {
  readonly trail: TrailKind;
  readonly pulse: PulseKind;
  /** How long each effect lasts against its own length. Left out, 1. */
  readonly length?: number | undefined;
  /** A plugin's shader for the trail, in place of the built-in one. */
  readonly customTrail?: CustomEffect | undefined;
  /** A plugin's shader for the pulse, in place of the built-in one. */
  readonly customPulse?: CustomEffect | undefined;
}

interface Last {
  readonly source: CursorSource;
  readonly col: number;
  readonly row: number;
  readonly shape: number;
}

const OFF: CursorFxSettings = { trail: "none", pulse: "none" };

const TRAILS: readonly string[] = ["none", "warp", "sweep", "comet"];
const PULSES: readonly string[] = ["none", "burst", "ripple", "box-burst", "box-ripple"];

/** A plugin's effect of a kind, by the id the settings name it with. */
function customOf(
  shown: UiContributions | null | undefined,
  id: string,
  kind: "trail" | "pulse",
): CustomEffect | undefined {
  const found = shown?.cursor_effects.find((effect) => effect.id === id && effect.kind === kind);
  return found
    ? { id: found.id, source: found.source, duration: found.duration, reach: found.reach }
    : undefined;
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
  return {
    trail: TRAILS.includes(trail) ? (trail as TrailKind) : "none",
    pulse: PULSES.includes(pulse) ? (pulse as PulseKind) : "none",
    ...(typeof length === "number" ? { length } : {}),
    ...(customTrail ? { customTrail } : {}),
    ...(customPulse ? { customPulse } : {}),
  };
}

/** Whether settings ask for any effect at all. */
function anyEffect(settings: CursorFxSettings): boolean {
  return (
    settings.trail !== "none" ||
    settings.pulse !== "none" ||
    settings.customTrail !== undefined ||
    settings.customPulse !== undefined
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

  /** Told when a plugin's effect cannot be drawn: its id and why. */
  onProblem(listener: (id: string, message: string) => void): void {
    this.#problem = listener;
    this.#effects?.onProblem?.(listener);
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
  }

  /** The window's size in CSS pixels, and its device pixel ratio. */
  resize(size: { width: number; height: number }, dpr: number): void {
    this.#size = size;
    this.#dpr = dpr;
    this.#effects?.resize(size, dpr);
  }

  report(source: CursorSource, report: CursorReport, now: number): void {
    const effects = this.#effects;
    if (!effects || !this.watching) return;
    // A cursor without the keyboard plays nothing, and is not where the keyboard goes
    // from: the pane that last had it is.
    if (!report.focused || !report.visible) return;
    const last = this.#last;
    this.#last = { source, col: report.col, row: report.row, shape: report.shape };
    if (!last) return;
    if (last.source !== source) {
      const from = last.source.cellRect(last.col, last.row);
      const to = source.cellRect(report.col, report.row);
      if (!to) return;
      if (from) effects.move(from, to, report.color, now);
      effects.pulse(to, report.color, now);
      return;
    }
    if (report.reflowed) return;
    const moved = last.col !== report.col || last.row !== report.row;
    const reshaped = last.shape !== report.shape;
    if (!moved && !reshaped) return;
    const to = source.cellRect(report.col, report.row);
    if (!to) return;
    if (moved) {
      const from = source.cellRect(last.col, last.row);
      if (from) effects.move(from, to, report.color, now);
    }
    if (reshaped) effects.pulse(to, report.color, now);
  }

  forget(source: CursorSource): void {
    if (this.#last?.source === source) this.#last = undefined;
  }

  #wanted(): CursorFxSettings {
    if (this.#reducedMotion?.matches === true) return OFF;
    if (!anyEffect(this.#settings)) return OFF;
    return this.#settings;
  }
}
