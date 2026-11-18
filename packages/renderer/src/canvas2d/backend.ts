/**
 * The Canvas 2D tier: the one that always works.
 *
 * It reads the same instance records as the GPU tiers and composes the picture itself,
 * pixel by pixel, in a buffer the size of the surface. The canvas is only where the
 * finished pixels are put.
 *
 * Composing by hand is what suits a last resort. A 2D context cannot tint an image as it
 * draws it, and a terminal is mostly tinted glyphs, so drawing through the context would
 * take several calls per glyph and tens of thousands per frame. Here a glyph is a short
 * loop over its pixels with the same arithmetic a GPU's blending does, so the picture
 * matches the other tiers, and the cost does not depend on whether the canvas itself is
 * backed by a GPU. This tier is used exactly where there may be none.
 *
 * The buffer keeps what was drawn, so only the rows that changed are composed again and
 * only their band of the canvas is updated.
 */

import type { GlyphAtlas } from "../atlas/atlas.ts";
import { ATLAS_PAGE_SIZE, LINE_RECTS_PER_CELL } from "../constants.ts";
import { GLYPH_FLAG_COLOR, type Plane } from "../layout.ts";
import { TierUnavailableError } from "../select.ts";
import type { AtlasSink, PageKind, PresentInfo, Tier, TierBackend } from "../types.ts";

/** Atlas pages kept as plain bytes. */
class PixelPages implements AtlasSink {
  /** One byte of coverage per pixel. */
  readonly mask: Uint8Array[] = [];
  /** Four bytes per pixel: red, green, blue, alpha, not premultiplied. */
  readonly color: Uint8Array[] = [];

  ensurePages(kind: PageKind, count: number): void {
    const pages = kind === "mask" ? this.mask : this.color;
    const bytes = ATLAS_PAGE_SIZE * ATLAS_PAGE_SIZE * (kind === "mask" ? 1 : 4);
    while (pages.length < count) pages.push(new Uint8Array(bytes));
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
    const target = (kind === "mask" ? this.mask : this.color)[page];
    if (!target) return;
    const size = kind === "mask" ? 1 : 4;
    const rowBytes = width * size;
    for (let row = 0; row < height; row++) {
      target.set(
        pixels.subarray(row * rowBytes, (row + 1) * rowBytes),
        ((y + row) * ATLAS_PAGE_SIZE + x) * size,
      );
    }
  }
}

/** Draws instance records by composing pixels and putting them on a 2D canvas. */
export class Canvas2dBackend implements TierBackend {
  readonly tier: Tier = "canvas2d";
  readonly #canvas: HTMLCanvasElement;
  readonly #context: CanvasRenderingContext2D;
  #atlasKey: string | undefined;
  #atlas: GlyphAtlas | undefined;
  #pages = new PixelPages();

  // The picture: the canvas's pixels as bytes, and the same memory as 32-bit words for
  // filling runs of one colour. A word holds a pixel exactly as colours are packed.
  // Blends are done in whole numbers, rounded to nearest, which is what a GPU arrives at.
  #image: ImageData | undefined;
  #bytes: Uint8ClampedArray = new Uint8ClampedArray(0);
  #words: Uint32Array = new Uint32Array(0);
  #width = 0;
  #height = 0;

  #lost: ((reason: string) => void) | undefined;
  readonly #onContextLost = () => this.#lost?.("the 2D context was lost");

  constructor(canvas: HTMLCanvasElement) {
    this.#canvas = canvas;
    const context = canvas.getContext("2d", { alpha: false });
    if (!context) throw new TierUnavailableError("canvas2d", "no 2D context");
    this.#context = context;
    canvas.addEventListener("contextlost", this.#onContextLost);
  }

  atlas(key: string, create: (sink: AtlasSink) => GlyphAtlas): GlyphAtlas {
    if (this.#atlas && this.#atlasKey === key) return this.#atlas;
    // Glyphs of another font or size are of no further use; let their pages go.
    this.#pages = new PixelPages();
    this.#atlas = create(this.#pages);
    this.#atlasKey = key;
    return this.#atlas;
  }

  configure(width: number, height: number): void {
    this.#canvas.width = width;
    this.#canvas.height = height;
    this.#image = new ImageData(width, height);
    this.#bytes = this.#image.data;
    this.#words = new Uint32Array(this.#bytes.buffer);
    this.#width = width;
    this.#height = height;
  }

  setPlanes(): void {
    // The planes are read where they are; there is no copy to keep.
  }

  upload(): void {
    // Nothing to send anywhere.
  }

  present(info: PresentInfo): void {
    const image = this.#image;
    if (!image) return;
    const { planes, cols, rows, metrics, repaint } = info;
    const { cellHeight, originY } = metrics;
    // The background is opaque whatever the colour says: nothing shows through a terminal.
    const clear = (info.clear | 0xff000000) >>> 0;

    if (!repaint) this.#words.fill(clear);
    for (let row = 0; row < rows; row++) {
      if (repaint && repaint[row] !== 1) continue;
      const y = originY + row * cellHeight;
      // Start the row again from the background, across the whole width.
      if (repaint) this.#fill(0, y, this.#width, cellHeight, clear);
      const first = row * cols;
      this.#rects(planes.background, first, cols);
      this.#glyphs(planes.underline, first, cols);
      this.#glyphs(planes.glyph, first, cols);
      this.#rects(planes.line, first * LINE_RECTS_PER_CELL, cols * LINE_RECTS_PER_CELL);
    }

    // The cursor goes on top. Its row is always among those composed again when it
    // changes, and composing it again over itself changes nothing.
    if (info.overlayRects > 0) this.#rects(planes.overlayRect, 0, info.overlayRects);
    if (info.overlayGlyphs > 0) this.#glyphs(planes.overlayGlyph, 0, info.overlayGlyphs);

    if (!repaint) {
      this.#context.putImageData(image, 0, 0);
      return;
    }
    // Update the canvas one band per run of neighbouring rows.
    for (let row = 0; row < rows; row++) {
      if (repaint[row] !== 1) continue;
      let end = row + 1;
      while (end < rows && repaint[end] === 1) end++;
      const top = originY + row * cellHeight;
      const height = Math.min(this.#height - top, (end - row) * cellHeight);
      if (height > 0) this.#context.putImageData(image, 0, 0, 0, top, this.#width, height);
      row = end;
    }
  }

  onLost(listener: (reason: string) => void): void {
    this.#lost = listener;
  }

  dispose(): void {
    this.#canvas.removeEventListener("contextlost", this.#onContextLost);
    this.#lost = undefined;
    this.#atlas = undefined;
    this.#pages = new PixelPages();
    this.#image = undefined;
    this.#bytes = new Uint8ClampedArray(0);
    this.#words = new Uint32Array(0);
  }

  /** Compose a run of rectangle records. */
  #rects(plane: Plane, first: number, count: number): void {
    const { i16, u16, u32 } = plane;
    for (let slot = first; slot < first + count; slot++) {
      const at = slot * 6;
      const width = u16[at + 2]!;
      if (width === 0) continue;
      this.#fill(i16[at]!, i16[at + 1]!, width, u16[at + 3]!, u32[slot * 3 + 2]!);
    }
  }

  /** Fill a rectangle, clipped to the surface, with a packed colour. */
  #fill(x: number, y: number, width: number, height: number, color: number): void {
    const x0 = Math.max(0, x);
    const y0 = Math.max(0, y);
    const x1 = Math.min(this.#width, x + width);
    const y1 = Math.min(this.#height, y + height);
    if (x1 <= x0 || y1 <= y0) return;
    const alpha = color >>> 24;
    if (alpha === 255) {
      for (let row = y0; row < y1; row++) {
        const start = row * this.#width;
        this.#words.fill(color, start + x0, start + x1);
      }
      return;
    }
    if (alpha === 0) return;
    const bytes = this.#bytes;
    const r = color & 0xff;
    const g = (color >>> 8) & 0xff;
    const b = (color >>> 16) & 0xff;
    const keep = 255 - alpha;
    for (let row = y0; row < y1; row++) {
      let at = (row * this.#width + x0) * 4;
      for (let col = x0; col < x1; col++, at += 4) {
        bytes[at] = ((r * alpha + bytes[at]! * keep + 127) / 255) | 0;
        bytes[at + 1] = ((g * alpha + bytes[at + 1]! * keep + 127) / 255) | 0;
        bytes[at + 2] = ((b * alpha + bytes[at + 2]! * keep + 127) / 255) | 0;
      }
    }
  }

  /** Compose a run of glyph records. */
  #glyphs(plane: Plane, first: number, count: number): void {
    const { i16, u16, u32 } = plane;
    for (let slot = first; slot < first + count; slot++) {
      const at = slot * 10;
      const width = u16[at + 2]!;
      if (width === 0) continue;
      const height = u16[at + 3]!;
      const u = u16[at + 4]!;
      const v = u16[at + 5]!;
      const page = u16[at + 6]!;
      if ((u16[at + 7]! & GLYPH_FLAG_COLOR) !== 0) {
        const pixels = this.#pages.color[page];
        if (pixels) this.#color(pixels, u, v, width, height, i16[at]!, i16[at + 1]!);
      } else {
        const pixels = this.#pages.mask[page];
        if (pixels) {
          this.#mask(pixels, u, v, width, height, i16[at]!, i16[at + 1]!, u32[slot * 5 + 4]!);
        }
      }
    }
  }

  /** Blend a coverage mask in one colour onto the picture at `x`, `y`. */
  #mask(
    page: Uint8Array,
    u: number,
    v: number,
    width: number,
    height: number,
    x: number,
    y: number,
    color: number,
  ): void {
    const x0 = Math.max(0, x);
    const y0 = Math.max(0, y);
    const x1 = Math.min(this.#width, x + width);
    const y1 = Math.min(this.#height, y + height);
    if (x1 <= x0 || y1 <= y0) return;

    const bytes = this.#bytes;
    const words = this.#words;
    const r = color & 0xff;
    const g = (color >>> 8) & 0xff;
    const b = (color >>> 16) & 0xff;
    const opaque = (color | 0xff000000) >>> 0;
    const alpha = color >>> 24;
    for (let row = y0; row < y1; row++) {
      let source = (v + row - y) * ATLAS_PAGE_SIZE + u + (x0 - x);
      let pixel = row * this.#width + x0;
      for (let col = x0; col < x1; col++, source++, pixel++) {
        let coverage = page[source]!;
        if (coverage === 0) continue;
        if (alpha !== 255) coverage = ((coverage * alpha + 127) / 255) | 0;
        if (coverage === 255) {
          words[pixel] = opaque;
          continue;
        }
        const keep = 255 - coverage;
        const at = pixel * 4;
        bytes[at] = ((r * coverage + bytes[at]! * keep + 127) / 255) | 0;
        bytes[at + 1] = ((g * coverage + bytes[at + 1]! * keep + 127) / 255) | 0;
        bytes[at + 2] = ((b * coverage + bytes[at + 2]! * keep + 127) / 255) | 0;
      }
    }
  }

  /** Blend a glyph that has its own colours onto the picture at `x`, `y`. */
  #color(
    page: Uint8Array,
    u: number,
    v: number,
    width: number,
    height: number,
    x: number,
    y: number,
  ): void {
    const x0 = Math.max(0, x);
    const y0 = Math.max(0, y);
    const x1 = Math.min(this.#width, x + width);
    const y1 = Math.min(this.#height, y + height);
    if (x1 <= x0 || y1 <= y0) return;

    const bytes = this.#bytes;
    for (let row = y0; row < y1; row++) {
      let source = ((v + row - y) * ATLAS_PAGE_SIZE + u + (x0 - x)) * 4;
      let at = (row * this.#width + x0) * 4;
      for (let col = x0; col < x1; col++, source += 4, at += 4) {
        const alpha = page[source + 3]!;
        if (alpha === 0) continue;
        const keep = 255 - alpha;
        bytes[at] = ((page[source]! * alpha + bytes[at]! * keep + 127) / 255) | 0;
        bytes[at + 1] = ((page[source + 1]! * alpha + bytes[at + 1]! * keep + 127) / 255) | 0;
        bytes[at + 2] = ((page[source + 2]! * alpha + bytes[at + 2]! * keep + 127) / 255) | 0;
      }
    }
  }
}
