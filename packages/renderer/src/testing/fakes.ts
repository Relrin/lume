/**
 * Stand-ins for the parts of the renderer that need a browser, so the rest can be
 * tested without one.
 */

import type { GlyphAtlas } from "../atlas/atlas.ts";
import { rgba } from "../frame.ts";
import type { PlaneName, Planes } from "../layout.ts";
import type {
  AtlasSink,
  CellMetrics,
  FontBackend,
  FontSpec,
  GlyphRasteriser,
  PageKind,
  PresentInfo,
  RasterGlyph,
  RawFontMetrics,
  TerminalTheme,
  Tier,
  TierBackend,
} from "../types.ts";

/** A theme for tests. */
export const TEST_THEME: TerminalTheme = {
  font: { family: "Test Mono", sizePx: 14 },
  cursor: rgba(0xff, 0xff, 0xff),
  selectionBackground: rgba(0x33, 0x55, 0x99),
  padding: 0,
};

/** One stored glyph, as a sink received it. */
export interface SinkUpload {
  kind: PageKind;
  page: number;
  x: number;
  y: number;
  width: number;
  height: number;
  pixels: Uint8Array;
}

/** An atlas sink that remembers what it was given. */
export class RecordingSink implements AtlasSink {
  readonly pages: Record<PageKind, number> = { mask: 0, color: 0 };
  readonly uploads: SinkUpload[] = [];

  ensurePages(kind: PageKind, count: number): void {
    this.pages[kind] = Math.max(this.pages[kind], count);
  }

  upload(
    kind: PageKind,
    page: number,
    x: number,
    y: number,
    width: number,
    height: number,
    pixels: Uint8Array,
  ): void {
    this.uploads.push({ kind, page, x, y, width, height, pixels });
  }
}

/**
 * A rasteriser that draws every glyph as a solid box a little smaller than its cells.
 * Spaces have no ink; emoji come back as colour.
 */
export class FakeRasteriser implements GlyphRasteriser {
  readonly calls: string[] = [];
  /** Extra pixels added to each glyph's size, to fill an atlas quickly in tests. */
  inflate = 0;

  constructor(readonly metrics: CellMetrics) {}

  rasterise(text: string, bold: boolean, italic: boolean, span: number): RasterGlyph {
    this.calls.push(`${text}${bold ? " bold" : ""}${italic ? " italic" : ""} x${span}`);
    if (text.trim() === "") {
      return {
        width: 0,
        height: 0,
        offsetX: 0,
        offsetY: 0,
        kind: "mask",
        pixels: new Uint8Array(),
      };
    }
    const width = Math.max(1, this.metrics.cellWidth * span - 2) + this.inflate;
    const height = Math.max(1, this.metrics.cellHeight - 4) + this.inflate;
    const color = (text.codePointAt(0) ?? 0) >= 0x1f300;
    return {
      width,
      height,
      offsetX: 1,
      offsetY: 2,
      kind: color ? "color" : "mask",
      pixels: new Uint8Array(width * height * (color ? 4 : 1)).fill(255),
    };
  }
}

/** Font services with fixed measurements: a cell of 8 by 16 pixels at a ratio of 1. */
export class FakeFonts implements FontBackend {
  readonly rasterisers: FakeRasteriser[] = [];
  measured = 0;

  measure(font: FontSpec, dpr: number): RawFontMetrics {
    this.measured++;
    const scale = (font.sizePx / 14) * dpr;
    return { advance: 8 * scale, ascent: 10.4 * scale, descent: 2.9 * scale };
  }

  rasteriser(_font: FontSpec, _dpr: number, metrics: CellMetrics): GlyphRasteriser {
    const rasteriser = new FakeRasteriser(metrics);
    this.rasterisers.push(rasteriser);
    return rasteriser;
  }
}

/** One upload a backend was asked for. */
export interface PlaneUpload {
  plane: PlaneName;
  start: number;
  end: number;
}

/** A tier backend that draws nothing and remembers what it was asked to do. */
export class RecordingBackend implements TierBackend {
  readonly sink = new RecordingSink();
  readonly atlases = new Map<string, GlyphAtlas>();
  readonly uploads: PlaneUpload[] = [];
  readonly presents: PresentInfo[] = [];
  readonly configured: [width: number, height: number][] = [];
  planes: Planes | undefined;
  planesSet = 0;
  disposed = false;
  #lost: ((reason: string) => void) | undefined;

  constructor(readonly tier: Tier = "canvas2d") {}

  atlas(key: string, create: (sink: AtlasSink) => GlyphAtlas): GlyphAtlas {
    let atlas = this.atlases.get(key);
    if (!atlas) {
      atlas = create(this.sink);
      this.atlases.set(key, atlas);
    }
    return atlas;
  }

  configure(width: number, height: number): void {
    this.configured.push([width, height]);
  }

  setPlanes(planes: Planes): void {
    this.planes = planes;
    this.planesSet++;
  }

  upload(plane: PlaneName, byteStart: number, byteEnd: number): void {
    this.uploads.push({ plane, start: byteStart, end: byteEnd });
  }

  present(info: PresentInfo): void {
    // The repaint array is reused between frames, so keep what it said now.
    this.presents.push({ ...info, repaint: info.repaint ? info.repaint.slice() : undefined });
  }

  onLost(listener: (reason: string) => void): void {
    this.#lost = listener;
  }

  dispose(): void {
    this.disposed = true;
  }

  /** Report the tier as lost, as a real one would after a device or context loss. */
  lose(reason = "test"): void {
    this.#lost?.(reason);
  }

  /** Forget the recorded uploads and presents. */
  clear(): void {
    this.uploads.length = 0;
    this.presents.length = 0;
  }

  /** The last frame presented. */
  get last(): PresentInfo {
    const last = this.presents.at(-1);
    if (!last) throw new Error("nothing was presented");
    return last;
  }
}
