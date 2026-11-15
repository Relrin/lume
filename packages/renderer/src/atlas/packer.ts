/**
 * Shelf packing: placing glyph rectangles into a square page.
 *
 * Glyphs of one font are all about one cell tall, so they pack well in rows ("shelves").
 * A rectangle goes on the first shelf that is tall enough and has room; if none does, a
 * new shelf is opened below the last one. Nothing is ever moved or removed: an atlas that
 * fills up is emptied and started again.
 */

interface Shelf {
  y: number;
  height: number;
  /** The x at which the next rectangle on this shelf starts. */
  next: number;
}

/** Where a rectangle was placed. */
export interface Placement {
  x: number;
  y: number;
}

/** Packs rectangles into one page. */
export class ShelfPacker {
  readonly #size: number;
  readonly #gap: number;
  readonly #shelves: Shelf[] = [];
  /** The y at which the next shelf would start. */
  #bottom = 0;

  /** `gap` is the empty space kept after every rectangle, to the right and below. */
  constructor(size: number, gap: number) {
    this.#size = size;
    this.#gap = gap;
  }

  /** Place a rectangle, or return `undefined` when the page has no room for it. */
  place(width: number, height: number): Placement | undefined {
    if (width > this.#size || height > this.#size) return undefined;

    // A shelf may be a little taller than the rectangle, but not so much taller that a
    // short glyph wastes a tall shelf while shorter shelves could still be opened.
    for (const shelf of this.#shelves) {
      const fits = height <= shelf.height && shelf.next + width <= this.#size;
      if (fits && shelf.height - height <= shelf.height >> 2) {
        const placed = { x: shelf.next, y: shelf.y };
        shelf.next += width + this.#gap;
        return placed;
      }
    }

    if (this.#bottom + height > this.#size) return undefined;
    const shelf: Shelf = { y: this.#bottom, height, next: width + this.#gap };
    this.#shelves.push(shelf);
    this.#bottom += height + this.#gap;
    return { x: 0, y: shelf.y };
  }

  /** Forget every placement. */
  clear(): void {
    this.#shelves.length = 0;
    this.#bottom = 0;
  }
}
