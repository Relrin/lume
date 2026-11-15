/**
 * The instance records every tier draws from, and the buffers that hold them.
 *
 * A frame is drawn as rectangles and glyphs. Each is one fixed-size record, and all three
 * tiers read the same bytes: the GPU tiers as per-instance vertex data, Canvas 2D by
 * walking the buffers. Coordinates are whole device pixels from the top-left corner of
 * the surface.
 *
 * Records live in planes with a fixed number of slots per cell, in row-major order. A row
 * is therefore one contiguous byte range of each plane, so a changed row is one upload,
 * and the number of instances drawn never changes with the content. A slot that is not
 * needed is all zero, which draws nothing.
 *
 * WebGPU requires every attribute offset and the record size to be multiples of four, so
 * both records are laid out in four-byte units.
 */

import { LINE_RECTS_PER_CELL, OVERLAY_GLYPH_CAPACITY, OVERLAY_RECT_CAPACITY } from "./constants.ts";

/** How one field of a record is read by a shader. */
export type FieldFormat = "sint16x2" | "uint16x2" | "unorm8x4";

/** One field of a record. */
export interface RecordField {
  readonly name: string;
  readonly offset: number;
  readonly format: FieldFormat;
}

/** The layout of a record kind. */
export interface RecordLayout {
  readonly stride: number;
  readonly fields: readonly RecordField[];
}

/** A filled rectangle: position, size, colour. */
export const RECT: RecordLayout = {
  stride: 12,
  fields: [
    { name: "position", offset: 0, format: "sint16x2" },
    { name: "size", offset: 4, format: "uint16x2" },
    { name: "color", offset: 8, format: "unorm8x4" },
  ],
};

/**
 * A piece of an atlas page drawn at a position: position, size, where in the page, which
 * page and what kind, colour. The piece is drawn at its own size, never scaled.
 */
export const GLYPH: RecordLayout = {
  stride: 20,
  fields: [
    { name: "position", offset: 0, format: "sint16x2" },
    { name: "size", offset: 4, format: "uint16x2" },
    { name: "texel", offset: 8, format: "uint16x2" },
    { name: "page", offset: 12, format: "uint16x2" },
    { name: "color", offset: 16, format: "unorm8x4" },
  ],
};

/** Bit of a glyph record's flags: the piece has its own colours and is not tinted. */
export const GLYPH_FLAG_COLOR = 1;

/** A buffer of records with the views needed to fill it. */
export interface Plane {
  readonly layout: RecordLayout;
  /** Records the plane holds. */
  readonly capacity: number;
  /** Records per cell, or 0 for a plane that is not laid out by cell. */
  readonly perCell: number;
  readonly bytes: Uint8Array<ArrayBuffer>;
  readonly i16: Int16Array<ArrayBuffer>;
  readonly u16: Uint16Array<ArrayBuffer>;
  readonly u32: Uint32Array<ArrayBuffer>;
}

/** The names of the planes, in the order they are drawn. */
export const PLANE_ORDER = [
  "background",
  "underline",
  "glyph",
  "line",
  "overlayRect",
  "overlayGlyph",
] as const;

/** The name of a plane. */
export type PlaneName = (typeof PLANE_ORDER)[number];

/** All the planes of one renderer. */
export type Planes = Readonly<Record<PlaneName, Plane>>;

function plane(layout: RecordLayout, capacity: number, perCell: number): Plane {
  const buffer = new ArrayBuffer(Math.max(capacity, 1) * layout.stride);
  return {
    layout,
    capacity,
    perCell,
    bytes: new Uint8Array(buffer),
    i16: new Int16Array(buffer),
    u16: new Uint16Array(buffer),
    u32: new Uint32Array(buffer),
  };
}

/** Planes for a grid of `cells` cells. */
export function createPlanes(cells: number): Planes {
  return {
    background: plane(RECT, cells, 1),
    underline: plane(GLYPH, cells, 1),
    glyph: plane(GLYPH, cells, 1),
    line: plane(RECT, cells * LINE_RECTS_PER_CELL, LINE_RECTS_PER_CELL),
    overlayRect: plane(RECT, OVERLAY_RECT_CAPACITY, 0),
    overlayGlyph: plane(GLYPH, OVERLAY_GLYPH_CAPACITY, 0),
  };
}

/** Write a rectangle into `slot` of a plane of rectangles. */
export function writeRect(
  target: Plane,
  slot: number,
  x: number,
  y: number,
  width: number,
  height: number,
  color: number,
): void {
  const at = slot * 6;
  target.i16[at] = x;
  target.i16[at + 1] = y;
  target.u16[at + 2] = width;
  target.u16[at + 3] = height;
  target.u32[slot * 3 + 2] = color;
}

/** Write a glyph into `slot` of a plane of glyphs. */
export function writeGlyph(
  target: Plane,
  slot: number,
  x: number,
  y: number,
  width: number,
  height: number,
  u: number,
  v: number,
  page: number,
  flags: number,
  color: number,
): void {
  const at = slot * 10;
  target.i16[at] = x;
  target.i16[at + 1] = y;
  target.u16[at + 2] = width;
  target.u16[at + 3] = height;
  target.u16[at + 4] = u;
  target.u16[at + 5] = v;
  target.u16[at + 6] = page;
  target.u16[at + 7] = flags;
  target.u32[slot * 5 + 4] = color;
}

/** Zero the records of a run of slots. */
export function clearSlots(target: Plane, first: number, count: number): void {
  const stride = target.layout.stride;
  target.bytes.fill(0, first * stride, (first + count) * stride);
}

/**
 * Whether this machine stores multi-byte numbers least significant byte first. The
 * records are written through typed-array views and read by shaders as little-endian, so
 * nothing here works otherwise. Every machine a webview runs on passes.
 */
export function isLittleEndian(): boolean {
  return new Uint8Array(new Uint16Array([1]).buffer)[0] === 1;
}
