/**
 * Working out which rows need rebuilding and which byte ranges need uploading.
 */

import { DirtyState, NO_SELECTION, type RenderFrame } from "./frame.ts";

/**
 * Mark the rows of a frame whose records must be rebuilt.
 *
 * That is the rows the frame flags, and also any row whose selected range differs from
 * the one it was last built with, because the selection changes a cell's colours without
 * the cell itself changing. `selectionStart` and `selectionEnd` hold the ranges of the
 * last build and are brought up to date.
 *
 * Returns how many rows are marked in `out`.
 */
export function collectDirtyRows(
  frame: RenderFrame,
  force: boolean,
  selectionStart: Int32Array,
  selectionEnd: Int32Array,
  out: Uint8Array,
): number {
  const full = force || frame.dirty === DirtyState.Full;
  let count = 0;
  for (let row = 0; row < frame.rows; row++) {
    const start = frame.selectionStart[row]!;
    const end = start === NO_SELECTION ? 0 : frame.selectionEnd[row]!;
    const selectionChanged = start !== selectionStart[row] || end !== selectionEnd[row];
    selectionStart[row] = start;
    selectionEnd[row] = end;

    const dirty = full || selectionChanged || frame.dirtyRows[row] === 1;
    out[row] = dirty ? 1 : 0;
    if (dirty) count++;
  }
  return count;
}

/**
 * The runs of consecutive marked rows, as pairs of first row and one past the last.
 * Neighbouring rows are neighbouring bytes in a plane, so each run is one upload.
 */
export function rowRuns(rows: Uint8Array, count: number): number[] {
  const runs: number[] = [];
  let start = -1;
  for (let row = 0; row < count; row++) {
    if (rows[row] === 1) {
      if (start < 0) start = row;
    } else if (start >= 0) {
      runs.push(start, row);
      start = -1;
    }
  }
  if (start >= 0) runs.push(start, count);
  return runs;
}
