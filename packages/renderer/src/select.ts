/**
 * Choosing a tier, and moving to the next one when a tier is lost.
 *
 * A GPU device or a WebGL context can be taken away at any time: a driver reset, the
 * machine waking from sleep, the browser reclaiming memory. The terminal must keep
 * showing its contents, so `ManagedRenderer` then starts the next tier in the order and
 * draws the same frame with it.
 *
 * A canvas is tied for good to the first kind of context created on it, so each tier
 * gets a canvas of its own. That is why the renderer, not its caller, owns the canvas:
 * the caller provides a place to put one and never sees it replaced.
 */

import { GridRenderer } from "./core.ts";
import { HIGHLIGHT_STRIDE, type Highlights, type RenderFrame } from "./frame.ts";
import type {
  CellMetrics,
  CellRenderer,
  FontBackend,
  Size,
  TerminalTheme,
  Tier,
  TierBackend,
  UnderlineRange,
} from "./types.ts";

/** A tier cannot be used here. The next one in the order is tried. */
export class TierUnavailableError extends Error {
  constructor(tier: Tier, why: string) {
    super(`${tier} is not available: ${why}`);
    this.name = "TierUnavailableError";
  }
}

/** Makes the backend of one tier on a canvas, or throws `TierUnavailableError`. */
export interface TierFactory<Canvas> {
  readonly tier: Tier;
  create(canvas: Canvas): TierBackend;
}

/** The place a renderer keeps its canvas. */
export interface Surface<Canvas> {
  createCanvas(): Canvas;
  /** Show this canvas, in place of any shown before. */
  show(canvas: Canvas): void;
  /** Take a canvas away, shown or not. */
  remove(canvas: Canvas): void;
  /** Set the size the canvas is laid out at, in CSS pixels. */
  setCssSize(canvas: Canvas, size: Size): void;
}

/** Choices made when a renderer is created. */
export interface RendererOptions {
  /** Use this tier and no other, for testing a tier or working round a bad one. */
  tier?: Tier;
  /** Told when a tier is lost and another has taken over, or none could. */
  onTierChange?(tier: Tier | undefined, reason: string): void;
  /**
   * Told when the renderer needs a frame drawn although the terminal has not changed,
   * as after losing a tier. The owner should ask its frame loop for a frame.
   */
  onInvalidate?(): void;
}

/** A renderer that survives the loss of its tier by starting the next one. */
export class ManagedRenderer<Canvas> implements CellRenderer {
  readonly #surface: Surface<Canvas>;
  readonly #factories: readonly TierFactory<Canvas>[];
  readonly #fonts: FontBackend;
  readonly #options: RendererOptions;
  #theme: TerminalTheme;
  #size: Size = { width: 0, height: 0 };
  #dpr = 1;
  #focused = true;
  #cursorVisible = true;
  #underlines: readonly UnderlineRange[] = [];
  /** The highlighted runs as last given, kept to hand to the tier that takes over. */
  readonly #highlights: Highlights = { data: new Int32Array(0), count: 0 };

  #inner: GridRenderer;
  #canvas: Canvas;
  /** Index into `#factories` of the tier in use. */
  #index: number;
  /** Why the tier in use stopped working, once it has. */
  #lost: string | undefined;
  #disposed = false;

  constructor(
    surface: Surface<Canvas>,
    factories: readonly TierFactory<Canvas>[],
    fonts: FontBackend,
    theme: TerminalTheme,
    options: RendererOptions = {},
  ) {
    this.#surface = surface;
    this.#fonts = fonts;
    this.#theme = theme;
    this.#options = options;
    this.#factories = options.tier
      ? factories.filter((factory) => factory.tier === options.tier)
      : factories;

    const started = this.#start(0);
    if (!started) {
      const tried = this.#factories.map((factory) => factory.tier).join(", ") || "none";
      throw new Error(`no renderer tier could be started (tried: ${tried})`);
    }
    this.#inner = started.inner;
    this.#canvas = started.canvas;
    this.#index = started.index;
    surface.show(started.canvas);
  }

  get tier(): Tier {
    return this.#inner.tier;
  }

  get metrics(): CellMetrics {
    return this.#inner.metrics;
  }

  resize(px: Size, dpr: number): void {
    this.#size = { width: px.width, height: px.height };
    this.#dpr = dpr;
    this.#surface.setCssSize(this.#canvas, this.#size);
    this.#inner.resize(px, dpr);
  }

  setTheme(theme: TerminalTheme): void {
    this.#theme = theme;
    this.#inner.setTheme(theme);
  }

  setFocused(focused: boolean): void {
    this.#focused = focused;
    this.#inner.setFocused(focused);
  }

  setCursorVisible(visible: boolean): void {
    this.#cursorVisible = visible;
    this.#inner.setCursorVisible(visible);
  }

  setUnderlines(ranges: readonly UnderlineRange[]): void {
    this.#underlines = ranges.map((range) => ({ ...range }));
    this.#inner.setUnderlines(this.#underlines);
  }

  setHighlights(highlights: Highlights | undefined): void {
    const count = highlights?.count ?? 0;
    const length = count * HIGHLIGHT_STRIDE;
    const kept = this.#highlights;
    if (kept.data.length < length) kept.data = new Int32Array(length);
    if (highlights) kept.data.set(highlights.data.subarray(0, length));
    kept.count = count;
    this.#inner.setHighlights(kept);
  }

  gridSize(): { cols: number; rows: number } {
    return this.#inner.gridSize();
  }

  draw(frame: RenderFrame): void {
    if (this.#disposed) return;
    if (this.#lost === undefined) {
      try {
        this.#inner.draw(frame);
        return;
      } catch (error) {
        // A tier that throws while drawing is as lost as one that says so.
        this.#lost = error instanceof Error ? error.message : String(error);
      }
    }
    this.#fallBack(frame);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#inner.dispose();
    this.#surface.remove(this.#canvas);
  }

  /**
   * Start the first tier at or after `from` that works. The canvas is created but not
   * shown: the caller shows it once it has something on it.
   */
  #start(from: number): { inner: GridRenderer; canvas: Canvas; index: number } | undefined {
    for (let index = from; index < this.#factories.length; index++) {
      const factory = this.#factories[index]!;
      const canvas = this.#surface.createCanvas();
      let backend: TierBackend;
      try {
        backend = factory.create(canvas);
      } catch (error) {
        this.#surface.remove(canvas);
        if (error instanceof TierUnavailableError) continue;
        throw error;
      }
      backend.onLost((reason) => {
        // Only the tier in use matters; one already replaced may report late.
        if (this.#disposed || this.#factories[this.#index] !== factory) return;
        if (this.#lost !== undefined) return;
        this.#lost = reason;
        this.#options.onInvalidate?.();
      });
      const inner = new GridRenderer(backend, this.#fonts, this.#theme);
      inner.setFocused(this.#focused);
      inner.setCursorVisible(this.#cursorVisible);
      inner.setUnderlines(this.#underlines);
      inner.setHighlights(this.#highlights);
      if (this.#size.width > 0 && this.#size.height > 0) {
        this.#surface.setCssSize(canvas, this.#size);
        inner.resize(this.#size, this.#dpr);
      }
      return { inner, canvas, index };
    }
    return undefined;
  }

  /** Replace a lost tier with the next one and draw the frame with it. */
  #fallBack(frame: RenderFrame): void {
    const reason = this.#lost ?? "lost";
    let from = this.#index + 1;
    for (;;) {
      const next = this.#start(from);
      if (!next) {
        // Nothing left to draw with. The last picture stays; say so once.
        if (!this.#disposed) this.#options.onTierChange?.(undefined, reason);
        this.#disposed = true;
        this.#inner.dispose();
        return;
      }
      try {
        // Draw before showing, so the terminal is never seen empty in between.
        next.inner.draw(frame);
      } catch {
        next.inner.dispose();
        this.#surface.remove(next.canvas);
        from = next.index + 1;
        continue;
      }
      const old = { inner: this.#inner, canvas: this.#canvas };
      this.#inner = next.inner;
      this.#canvas = next.canvas;
      this.#index = next.index;
      this.#lost = undefined;
      this.#surface.show(next.canvas);
      old.inner.dispose();
      this.#surface.remove(old.canvas);
      this.#options.onTierChange?.(next.inner.tier, reason);
      return;
    }
  }
}
