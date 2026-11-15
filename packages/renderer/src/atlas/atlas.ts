/**
 * The glyph atlas: every glyph on screen, drawn once and kept.
 *
 * Drawing text is slow and the same few hundred glyphs repeat, so each is turned into
 * pixels the first time it is needed and stored in a page. Drawing a cell then means
 * copying a small rectangle of a page.
 *
 * The atlas decides where glyphs go and remembers where they went. Where the pixels are
 * kept is the tier's business: textures on a GPU, canvases otherwise. That is the
 * `AtlasSink`.
 *
 * A glyph is addressed by a slot number. Slot 0 is the empty glyph, which draws nothing:
 * spaces, and anything that could not be stored.
 *
 * Glyphs are never removed one by one. When the pages are full the atlas is emptied and
 * its `generation` changes; whoever holds slot numbers must then build its glyphs again,
 * which refills the atlas with what is actually on screen.
 */

import { ATLAS_GLYPH_GAP, ATLAS_MAX_PAGES, ATLAS_PAGE_SIZE } from "../constants.ts";
import { boxShapes, isProcedural } from "../procedural/box.ts";
import { rasterise } from "../procedural/coverage.ts";
import { underlineSprite } from "../procedural/underline.ts";
import type { AtlasSink, CellMetrics, GlyphRasteriser, PageKind, RasterGlyph } from "../types.ts";
import { ShelfPacker } from "./packer.ts";

/** The slot of a glyph that draws nothing. */
export const EMPTY_SLOT = 0;

/** Slots the atlas starts with room for. */
const INITIAL_SLOTS = 1024;

// Bits added to a codepoint to make the key of a glyph variant.
const KEY_BOLD = 1 << 21;
const KEY_ITALIC = 1 << 22;
const KEY_WIDE = 1 << 23;
const KEY_UNDERLINE = 1 << 24;

/** Glyphs drawn once and kept in pages. */
export class GlyphAtlas {
  /** Changes whenever the atlas is emptied. Slot numbers from before are then void. */
  generation = 0;

  // Where each slot is and how to place it, indexed by slot number. The arrays are
  // replaced when they grow, so they are read through the atlas each time.
  /** Position of the glyph in its page. */
  u = new Uint16Array(INITIAL_SLOTS);
  v = new Uint16Array(INITIAL_SLOTS);
  width = new Uint16Array(INITIAL_SLOTS);
  height = new Uint16Array(INITIAL_SLOTS);
  /** From the top-left corner of the glyph's first cell to where it is drawn. */
  offsetX = new Int16Array(INITIAL_SLOTS);
  offsetY = new Int16Array(INITIAL_SLOTS);
  page = new Uint8Array(INITIAL_SLOTS);
  /** 1 when the glyph has its own colours. */
  color = new Uint8Array(INITIAL_SLOTS);

  readonly #sink: AtlasSink;
  readonly #rasteriser: GlyphRasteriser;
  readonly #metrics: CellMetrics;
  readonly #byCodepoint = new Map<number, number>();
  readonly #byCluster = new Map<string, number>();
  readonly #packers: Record<PageKind, ShelfPacker[]> = { mask: [], color: [] };
  /** The next unused slot. Slot 0 is the empty glyph. */
  #next = 1;

  constructor(sink: AtlasSink, rasteriser: GlyphRasteriser, metrics: CellMetrics) {
    this.#sink = sink;
    this.#rasteriser = rasteriser;
    this.#metrics = metrics;
  }

  /** How many glyphs are stored. */
  get size(): number {
    return this.#next - 1;
  }

  /** How many pages of a kind are in use. */
  pages(kind: PageKind): number {
    return this.#packers[kind].length;
  }

  /** The slot of a single codepoint. `span` is the cells it occupies, 1 or 2. */
  glyph(codepoint: number, bold: boolean, italic: boolean, span: number): number {
    // Shapes drawn from the cell size look the same in every weight and slant.
    const procedural = isProcedural(codepoint);
    const key =
      codepoint |
      (bold && !procedural ? KEY_BOLD : 0) |
      (italic && !procedural ? KEY_ITALIC : 0) |
      (span > 1 ? KEY_WIDE : 0);
    const known = this.#byCodepoint.get(key);
    if (known !== undefined) return known;

    let raster: RasterGlyph;
    if (procedural) {
      const { cellWidth, cellHeight, lineThickness } = this.#metrics;
      const width = cellWidth * span;
      const shapes = boxShapes(codepoint, width, cellHeight, lineThickness) ?? [];
      raster = {
        width,
        height: cellHeight,
        offsetX: 0,
        offsetY: 0,
        kind: "mask",
        pixels: rasterise(shapes, width, cellHeight).pixels,
      };
    } else {
      raster = this.#rasteriser.rasterise(String.fromCodePoint(codepoint), bold, italic, span);
    }
    const slot = this.#store(raster);
    // Storing may have emptied the atlas, and with it the table this key goes into.
    this.#byCodepoint.set(key, slot);
    return slot;
  }

  /** The slot of a cell holding several codepoints. */
  cluster(text: string, bold: boolean, italic: boolean, span: number): number {
    const key = `${(bold ? 1 : 0) | (italic ? 2 : 0) | (span > 1 ? 4 : 0)}${text}`;
    const known = this.#byCluster.get(key);
    if (known !== undefined) return known;
    const slot = this.#store(this.#rasteriser.rasterise(text, bold, italic, span));
    this.#byCluster.set(key, slot);
    return slot;
  }

  /** The slot of an underline in the given style, one cell wide. */
  underline(style: number): number {
    const key = KEY_UNDERLINE | style;
    const known = this.#byCodepoint.get(key);
    if (known !== undefined) return known;

    const sprite = underlineSprite(style, this.#metrics);
    const slot = sprite
      ? this.#store({
          width: this.#metrics.cellWidth,
          height: sprite.height,
          offsetX: 0,
          offsetY: sprite.top,
          kind: "mask",
          pixels: rasterise(sprite.shapes, this.#metrics.cellWidth, sprite.height).pixels,
        })
      : EMPTY_SLOT;
    this.#byCodepoint.set(key, slot);
    return slot;
  }

  /** Forget every glyph. Slot numbers handed out before are void. */
  reset(): void {
    this.#byCodepoint.clear();
    this.#byCluster.clear();
    for (const packer of this.#packers.mask) packer.clear();
    for (const packer of this.#packers.color) packer.clear();
    this.#next = 1;
    this.generation++;
  }

  #store(raster: RasterGlyph): number {
    if (raster.width === 0 || raster.height === 0) return EMPTY_SLOT;
    let placed = this.#place(raster.kind, raster.width, raster.height);
    if (!placed) {
      // Every page is full. Start again; what is on screen will be stored anew.
      this.reset();
      placed = this.#place(raster.kind, raster.width, raster.height);
      // A glyph larger than a whole page cannot be stored at all.
      if (!placed) return EMPTY_SLOT;
    }

    const slot = this.#next++;
    if (slot >= this.u.length) this.#grow();
    this.u[slot] = placed.x;
    this.v[slot] = placed.y;
    this.width[slot] = raster.width;
    this.height[slot] = raster.height;
    this.offsetX[slot] = raster.offsetX;
    this.offsetY[slot] = raster.offsetY;
    this.page[slot] = placed.page;
    this.color[slot] = raster.kind === "color" ? 1 : 0;
    this.#sink.upload(
      raster.kind,
      placed.page,
      placed.x,
      placed.y,
      raster.width,
      raster.height,
      raster.pixels,
    );
    return slot;
  }

  #place(
    kind: PageKind,
    width: number,
    height: number,
  ): { x: number; y: number; page: number } | undefined {
    const packers = this.#packers[kind];
    for (let page = 0; page < packers.length; page++) {
      const at = packers[page]!.place(width, height);
      if (at) return { ...at, page };
    }
    if (packers.length >= ATLAS_MAX_PAGES) return undefined;
    const packer = new ShelfPacker(ATLAS_PAGE_SIZE, ATLAS_GLYPH_GAP);
    const at = packer.place(width, height);
    if (!at) return undefined;
    packers.push(packer);
    this.#sink.ensurePages(kind, packers.length);
    return { ...at, page: packers.length - 1 };
  }

  #grow(): void {
    const grow = <T extends Uint16Array | Int16Array | Uint8Array>(from: T): T => {
      const to = new (from.constructor as new (length: number) => T)(from.length * 2);
      to.set(from);
      return to;
    };
    this.u = grow(this.u);
    this.v = grow(this.v);
    this.width = grow(this.width);
    this.height = grow(this.height);
    this.offsetX = grow(this.offsetX);
    this.offsetY = grow(this.offsetY);
    this.page = grow(this.page);
    this.color = grow(this.color);
  }
}
