/**
 * Measuring fonts and drawing glyphs with a 2D canvas.
 *
 * Every tier gets its glyphs this way, so text is shaped and antialiased by the browser
 * and looks the same whichever tier draws it.
 *
 * The canvas is an ordinary element that is never added to the page. An element sees the
 * fonts loaded into the document without any doubt, and behaves the same in every
 * webview.
 */

import { DEFAULT_LINE_HEIGHT, GLYPH_OVERHANG_RATIO, MAX_CELL_SPAN } from "../constants.ts";
import type {
  CellMetrics,
  FontBackend,
  FontSpec,
  GlyphRasteriser,
  RasterGlyph,
  RawFontMetrics,
} from "../types.ts";
import { classify } from "./classify.ts";

const NORMAL_WEIGHT = 400;
const BOLD_WEIGHT = 700;

/** The CSS `font` value for a face of a font at a device pixel ratio. */
function cssFont(font: FontSpec, dpr: number, bold: boolean, italic: boolean): string {
  const weight = bold ? (font.boldWeight ?? BOLD_WEIGHT) : (font.weight ?? NORMAL_WEIGHT);
  // Drawing at the device size is what keeps text sharp on a dense display.
  return `${italic ? "italic " : ""}${weight} ${font.sizePx * dpr}px ${font.family}`;
}

function context2d(canvas: HTMLCanvasElement): CanvasRenderingContext2D {
  // The pixels are read back for every glyph, so they should stay in ordinary memory.
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) throw new Error("a 2D canvas context could not be created");
  return context;
}

/** Draws glyphs of one font at one size. */
class CanvasRasteriser implements GlyphRasteriser {
  readonly #font: FontSpec;
  readonly #dpr: number;
  readonly #metrics: CellMetrics;
  readonly #context: CanvasRenderingContext2D;
  readonly #width: number;
  /** Room on each side of the glyph's cells for strokes that lean out of them. */
  readonly #margin: number;

  constructor(font: FontSpec, dpr: number, metrics: CellMetrics) {
    this.#font = font;
    this.#dpr = dpr;
    this.#metrics = metrics;
    this.#margin = Math.ceil(metrics.cellWidth * GLYPH_OVERHANG_RATIO);
    this.#width = metrics.cellWidth * MAX_CELL_SPAN + 2 * this.#margin;
    const canvas = document.createElement("canvas");
    canvas.width = this.#width;
    // Exactly one cell tall: a glyph that reaches into the row above or below is cut
    // off, so drawing one row never needs its neighbours redrawn.
    canvas.height = metrics.cellHeight;
    this.#context = context2d(canvas);
  }

  rasterise(text: string, bold: boolean, italic: boolean, span: number): RasterGlyph {
    const context = this.#context;
    const { cellWidth, cellHeight, baseline } = this.#metrics;
    context.clearRect(0, 0, this.#width, cellHeight);
    context.font = cssFont(this.#font, this.#dpr, bold, italic);
    context.textBaseline = "alphabetic";
    // White, so that what comes back in any other colour is the glyph's own.
    context.fillStyle = "#fff";

    const room = cellWidth * span;
    const advance = context.measureText(text).width;
    if (advance > room + 0.5) {
      // Wider than its cells, as emoji and glyphs from a fallback font often are:
      // shrink it to fit, evenly both ways so it keeps its shape, about the middle of
      // the cell's height so it stays centred on the line.
      const scale = room / advance;
      context.save();
      context.translate(this.#margin, cellHeight / 2);
      context.scale(scale, scale);
      context.fillText(text, 0, baseline - cellHeight / 2);
      context.restore();
    } else {
      // Narrower than a double-width cell pair: centre it. A single cell is the font's
      // own width, so there is nothing to centre.
      const inset = span > 1 ? Math.round((room - advance) / 2) : 0;
      context.fillText(text, this.#margin + inset, baseline);
    }

    const image = context.getImageData(0, 0, this.#width, cellHeight);
    const ink = classify(image.data, this.#width, cellHeight);
    return {
      width: ink.width,
      height: ink.height,
      offsetX: ink.x - this.#margin,
      offsetY: ink.y,
      kind: ink.kind,
      pixels: ink.pixels,
    };
  }
}

let measuring: CanvasRenderingContext2D | undefined;

/** Font measurement and glyph drawing through the document's 2D canvas. */
export const canvasFonts: FontBackend = {
  measure(font: FontSpec, dpr: number): RawFontMetrics {
    measuring ??= context2d(document.createElement("canvas"));
    measuring.font = cssFont(font, dpr, false, false);
    const measured = measuring.measureText("M");
    // The font's own ascent and descent, which do not depend on the letter measured.
    // An engine without them reports the letter's, which is a usable approximation.
    const ascent = measured.fontBoundingBoxAscent ?? measured.actualBoundingBoxAscent;
    const descent = measured.fontBoundingBoxDescent ?? measured.actualBoundingBoxDescent;
    const size = font.sizePx * dpr;
    return {
      advance: measured.width > 0 ? measured.width : size * 0.6,
      ascent: ascent > 0 ? ascent : size * 0.8,
      descent: descent > 0 ? descent : (size * (DEFAULT_LINE_HEIGHT - 1)) / 2,
    };
  },

  rasteriser(font: FontSpec, dpr: number, metrics: CellMetrics): GlyphRasteriser {
    return new CanvasRasteriser(font, dpr, metrics);
  },
};
