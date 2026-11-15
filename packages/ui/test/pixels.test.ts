import { describe, expect, it } from "vitest";

import { compare, differingBlocks } from "../src/harness/pixels.ts";
import { REFERENCE_BLOCK_TOLERANCE } from "../src/harness/tuning.ts";

/** A picture of one colour, as the comparisons take it. */
function picture(width: number, height: number, level: number): ImageData {
  const data = new Uint8ClampedArray(width * height * 4);
  for (let i = 0; i < data.length; i += 4) {
    data[i] = level;
    data[i + 1] = level;
    data[i + 2] = level;
    data[i + 3] = 255;
  }
  return { width, height, data, colorSpace: "srgb" } as ImageData;
}

/** Sets one pixel's colour channels. */
function paint(target: ImageData, x: number, y: number, level: number): void {
  const at = (y * target.width + x) * 4;
  target.data[at] = level;
  target.data[at + 1] = level;
  target.data[at + 2] = level;
}

describe("comparing two pictures pixel by pixel", () => {
  it("finds none different in the same picture", () => {
    const a = picture(8, 8, 40);
    expect(compare(a, picture(8, 8, 40), 2)).toEqual({
      pixels: 64,
      exact: 0,
      beyondTolerance: 0,
      sameSize: true,
    });
  });

  it("counts a difference within the tolerance as different but not as beyond it", () => {
    const a = picture(8, 8, 40);
    const b = picture(8, 8, 40);
    paint(b, 1, 1, 42);
    paint(b, 2, 2, 43);
    expect(compare(a, b, 2)).toMatchObject({ exact: 2, beyondTolerance: 1 });
  });

  it("says when the pictures are not the same size", () => {
    const difference = compare(picture(8, 8, 0), picture(8, 9, 0), 2);
    expect(difference.sameSize).toBe(false);
    expect(difference.beyondTolerance).toBe(difference.pixels);
  });
});

describe("comparing a picture with its reference block by block", () => {
  it("finds no block different in the same picture", () => {
    expect(differingBlocks(picture(16, 16, 40), picture(16, 16, 40))).toBe(0);
  });

  it("lets the edges of letters be a little softer or sharper", () => {
    const reference = picture(16, 16, 40);
    const drawn = picture(16, 16, 40);
    // Every pixel a few levels off: the kind of difference a rasteriser makes.
    for (let y = 0; y < 16; y++) {
      for (let x = 0; x < 16; x++) paint(drawn, x, y, 40 + REFERENCE_BLOCK_TOLERANCE - 4);
    }
    expect(differingBlocks(drawn, reference)).toBe(0);
  });

  it("notices a letter that is missing", () => {
    const reference = picture(16, 16, 20);
    const drawn = picture(16, 16, 20);
    // A bright stroke three pixels wide through one block of the reference only.
    for (let y = 4; y < 8; y++) {
      for (let x = 4; x < 7; x++) paint(reference, x, y, 230);
    }
    // One block of sixteen differs.
    expect(differingBlocks(drawn, reference)).toBeCloseTo(1 / 16);
  });

  it("treats pictures of different sizes as wholly different", () => {
    expect(differingBlocks(picture(16, 16, 0), picture(20, 16, 0))).toBe(1);
  });
});
