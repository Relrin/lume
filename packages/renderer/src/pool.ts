/**
 * The renderers of one window, counted.
 *
 * A window makes a renderer for each pane it shows and lets it go when the pane is
 * hidden or closed. Some of what a renderer draws with is scarce: a browser keeps only
 * so many WebGL contexts alive in a page and takes the oldest away past its limit. So
 * the window keeps count of what its renderers hold, refuses a tier that has no room
 * left, in which case the pane takes the next tier, and can say how much is in use.
 *
 * Nothing here touches the page: the canvases, the tiers and the fonts are handed in,
 * which is also what lets this be tested without a browser.
 */

import type { GlyphAtlas } from "./atlas/atlas.ts";
import { ATLAS_BYTES_PER_PIXEL, ATLAS_PAGE_SIZE, WEBGL2_CONTEXTS_MAX } from "./constants.ts";
import type { PlaneName, Planes } from "./layout.ts";
import { themeMetrics } from "./metrics.ts";
import {
  ManagedRenderer,
  TierUnavailableError,
  type RendererOptions,
  type Surface,
  type TierFactory,
} from "./select.ts";
import type {
  AtlasSink,
  CellMetrics,
  CellRenderer,
  FontBackend,
  PageKind,
  PresentInfo,
  TerminalTheme,
  Tier,
  TierBackend,
} from "./types.ts";

/** What a window's renderers hold at one moment. */
export interface RenderStats {
  /** The renderers alive, by the tier each draws with. */
  renderers: Record<Tier, number>;
  /** The WebGL2 contexts alive. One that was lost is not counted. */
  webgl2Contexts: number;
  /**
   * The glyph atlases alive. Panes on a tier that shares textures count one for each
   * font and size between them; panes on the other tiers count one each.
   */
  atlases: number;
  /** About how many bytes the pages of those atlases take. */
  atlasBytes: number;
}

/** Makes the backend of each tier on a canvas, or throws `TierUnavailableError`. */
export type TierMakers<Canvas> = Readonly<Record<Tier, (canvas: Canvas) => TierBackend>>;

/** The most backends of a tier that may be alive at once, for the tiers that have a limit. */
export type TierLimits = Readonly<Partial<Record<Tier, number>>>;

/** The limits a window keeps to unless it is told otherwise. */
export const DEFAULT_TIER_LIMITS: TierLimits = { webgl2: WEBGL2_CONTEXTS_MAX };

const PAGE_KINDS: readonly PageKind[] = ["mask", "color"];

/**
 * A backend the pool keeps count of. It passes everything on to the tier's own backend
 * and notes three things on the way: the atlas the backend draws from, the loss of its
 * tier, and its end.
 */
class TrackedBackend implements TierBackend {
  readonly #inner: TierBackend;
  readonly #forget: (backend: TrackedBackend) => void;
  /** The atlas the backend last handed out. */
  atlasInUse: GlyphAtlas | undefined;
  /** Whether the tier can still draw. A lost one only waits to be replaced. */
  working = true;

  constructor(inner: TierBackend, forget: (backend: TrackedBackend) => void) {
    this.#inner = inner;
    this.#forget = forget;
    // Heard here even if nobody else asks to hear of it.
    inner.onLost(() => (this.working = false));
  }

  get tier(): Tier {
    return this.#inner.tier;
  }

  atlas(key: string, create: (sink: AtlasSink) => GlyphAtlas): GlyphAtlas {
    this.atlasInUse = this.#inner.atlas(key, create);
    return this.atlasInUse;
  }

  configure(width: number, height: number): void {
    this.#inner.configure(width, height);
  }

  setPlanes(planes: Planes): void {
    this.#inner.setPlanes(planes);
  }

  upload(plane: PlaneName, byteStart: number, byteEnd: number): void {
    this.#inner.upload(plane, byteStart, byteEnd);
  }

  present(info: PresentInfo): void {
    this.#inner.present(info);
  }

  onLost(listener: (reason: string) => void): void {
    this.#inner.onLost((reason) => {
      this.working = false;
      listener(reason);
    });
  }

  dispose(): void {
    this.working = false;
    this.atlasInUse = undefined;
    this.#forget(this);
    this.#inner.dispose();
  }
}

/** Makes renderers and keeps count of what they hold. */
export class RenderPool<Canvas> {
  readonly #order: readonly Tier[];
  readonly #makers: TierMakers<Canvas>;
  readonly #fonts: FontBackend;
  readonly #limits: TierLimits;
  /** The backends that have not been disposed. */
  readonly #alive = new Set<TrackedBackend>();

  /**
   * `order` is the tiers a renderer tries, first to last. `makers` makes each tier's
   * backend, `fonts` measures and draws glyphs, and `limits` says how many backends of
   * a tier may be alive at once.
   */
  constructor(
    order: readonly Tier[],
    makers: TierMakers<Canvas>,
    fonts: FontBackend,
    limits: TierLimits = DEFAULT_TIER_LIMITS,
  ) {
    this.#order = order;
    this.#makers = makers;
    this.#fonts = fonts;
    this.#limits = limits;
  }

  /**
   * Make a renderer that keeps its canvas on `surface`. A tier that has as many
   * backends alive as it may have is passed over, as one that is not available is.
   */
  createRenderer(
    surface: Surface<Canvas>,
    theme: TerminalTheme,
    options: RendererOptions = {},
  ): CellRenderer {
    const factories: TierFactory<Canvas>[] = this.#order.map((tier) => ({
      tier,
      create: (canvas) => this.#create(tier, canvas),
    }));
    return new ManagedRenderer(surface, factories, this.#fonts, theme, options);
  }

  /**
   * The cell geometry a renderer made here with `theme` reports at the device pixel
   * ratio `dpr`. No renderer and no surface is made, so the grid of a pane that does
   * not exist yet can be worked out from this.
   */
  measureCells(theme: TerminalTheme, dpr: number): CellMetrics {
    return themeMetrics(this.#fonts, theme, dpr);
  }

  /** What the renderers made here hold now. */
  stats(): RenderStats {
    const renderers: Record<Tier, number> = { webgpu: 0, webgl2: 0, canvas2d: 0 };
    // An atlas several panes share is one atlas, however many draw from it.
    const atlases = new Set<GlyphAtlas>();
    for (const backend of this.#alive) {
      renderers[backend.tier]++;
      if (backend.atlasInUse) atlases.add(backend.atlasInUse);
    }
    let atlasBytes = 0;
    for (const atlas of atlases) {
      for (const kind of PAGE_KINDS) {
        atlasBytes +=
          atlas.pages(kind) * ATLAS_PAGE_SIZE * ATLAS_PAGE_SIZE * ATLAS_BYTES_PER_PIXEL[kind];
      }
    }
    return {
      renderers,
      webgl2Contexts: this.#working("webgl2"),
      atlases: atlases.size,
      atlasBytes,
    };
  }

  /** How many backends of a tier are alive and can still draw. */
  #working(tier: Tier): number {
    let count = 0;
    for (const backend of this.#alive) {
      if (backend.tier === tier && backend.working) count++;
    }
    return count;
  }

  #create(tier: Tier, canvas: Canvas): TierBackend {
    const limit = this.#limits[tier];
    if (limit !== undefined && this.#working(tier) >= limit) {
      throw new TierUnavailableError(tier, `the window already has ${limit} in use`);
    }
    const backend = new TrackedBackend(this.#makers[tier](canvas), (ended) =>
      this.#alive.delete(ended),
    );
    this.#alive.add(backend);
    return backend;
  }
}
