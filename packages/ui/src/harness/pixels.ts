/**
 * Pictures as pixels: reading what a canvas shows, comparing two pictures, and turning
 * one into a PNG and back.
 */

import { REFERENCE_BLOCK_PIXELS, REFERENCE_BLOCK_TOLERANCE } from "./tuning.ts";

/** How two pictures of the same size differ. */
export interface Difference {
  /** Pixels in each picture. */
  readonly pixels: number;
  /** Pixels that are not identical. */
  readonly exact: number;
  /** Pixels with a colour channel further apart than the tolerance. */
  readonly beyondTolerance: number;
  /** Whether the pictures have the same size. When not, nothing else was compared. */
  readonly sameSize: boolean;
}

function scratch(width: number, height: number): CanvasRenderingContext2D {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) throw new Error("no 2D context for working on pixels");
  return context;
}

/** A copy of what a canvas shows right now. Call it in the task that drew the canvas. */
export function readPixels(canvas: HTMLCanvasElement): ImageData {
  const context = scratch(canvas.width, canvas.height);
  context.drawImage(canvas, 0, 0);
  return context.getImageData(0, 0, canvas.width, canvas.height);
}

/**
 * Compares two pictures channel by channel. Colour channels that differ by no more than
 * `tolerance` count as equal: blending on a GPU is not exact to the last level.
 */
export function compare(a: ImageData, b: ImageData, tolerance: number): Difference {
  const pixels = a.width * a.height;
  if (a.width !== b.width || a.height !== b.height) {
    return { pixels, exact: pixels, beyondTolerance: pixels, sameSize: false };
  }
  let exact = 0;
  let beyond = 0;
  for (let i = 0; i < a.data.length; i += 4) {
    const delta = Math.max(
      Math.abs(a.data[i]! - b.data[i]!),
      Math.abs(a.data[i + 1]! - b.data[i + 1]!),
      Math.abs(a.data[i + 2]! - b.data[i + 2]!),
    );
    if (delta > 0) exact++;
    if (delta > tolerance) beyond++;
  }
  return { pixels, exact, beyondTolerance: beyond, sameSize: true };
}

/**
 * Compares a picture with its reference block by block. A block differs when the mean
 * of any colour channel over it is further from the reference's than the tolerance.
 * Returns the share of blocks that differ, or 1 when the pictures are not the same size.
 */
export function differingBlocks(picture: ImageData, reference: ImageData): number {
  if (picture.width !== reference.width || picture.height !== reference.height) return 1;
  const size = REFERENCE_BLOCK_PIXELS;
  let blocks = 0;
  let differing = 0;
  for (let top = 0; top < picture.height; top += size) {
    for (let left = 0; left < picture.width; left += size) {
      const sums = [0, 0, 0];
      const wanted = [0, 0, 0];
      let count = 0;
      for (let y = top; y < Math.min(top + size, picture.height); y++) {
        for (let x = left; x < Math.min(left + size, picture.width); x++) {
          const at = (y * picture.width + x) * 4;
          for (let channel = 0; channel < 3; channel++) {
            sums[channel]! += picture.data[at + channel]!;
            wanted[channel]! += reference.data[at + channel]!;
          }
          count++;
        }
      }
      blocks++;
      const off = sums.some(
        (sum, channel) => Math.abs(sum - wanted[channel]!) / count > REFERENCE_BLOCK_TOLERANCE,
      );
      if (off) differing++;
    }
  }
  return blocks === 0 ? 0 : differing / blocks;
}

/** The picture as the bytes of a PNG file. */
export async function toPng(picture: ImageData): Promise<Uint8Array> {
  const context = scratch(picture.width, picture.height);
  context.putImageData(picture, 0, 0);
  const blob = await new Promise<Blob | null>((resolve) =>
    context.canvas.toBlob(resolve, "image/png"),
  );
  if (!blob) throw new Error("the picture could not be encoded");
  return new Uint8Array(await blob.arrayBuffer());
}

/** The picture in a PNG file, exactly as stored: no colour management, no premultiplying. */
export async function fromPng(bytes: Uint8Array): Promise<ImageData> {
  const blob = new Blob([bytes as BlobPart], { type: "image/png" });
  const bitmap = await createImageBitmap(blob, {
    colorSpaceConversion: "none",
    premultiplyAlpha: "none",
  });
  const context = scratch(bitmap.width, bitmap.height);
  context.drawImage(bitmap, 0, 0);
  bitmap.close();
  return context.getImageData(0, 0, context.canvas.width, context.canvas.height);
}
