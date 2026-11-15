/**
 * Finding the link under a cell.
 *
 * A link is either one a program marked as such, or an address written out in the text.
 * Both come from output, so what is found here is only a candidate: it is opened when the
 * user asks, after the target has been shown, and only if its kind is on the list.
 */

import { CellFlag, GLYPH_CLUSTER_TAG, type RenderFrame, type UnderlineRange } from "@lume/renderer";

import { LINK_SCHEMES, type LinkScheme } from "../limits.ts";
import { LINK_SCAN_MAX_ROWS } from "../tuning.ts";

/** A link and the cells that show it. */
export interface LinkHit {
  /** The address, exactly as the program wrote it. */
  target: string;
  ranges: UnderlineRange[];
}

/**
 * The kind of a link, if it is one the shell opens. The check is on the text itself and
 * deliberately simple: the core checks again, with a real parser, before opening anything.
 */
export function linkScheme(target: string): LinkScheme | null {
  const match = /^([a-z][a-z0-9+.-]*):/i.exec(target);
  if (!match) return null;
  const scheme = match[1]!.toLowerCase();
  return (LINK_SCHEMES as readonly string[]).includes(scheme) ? (scheme as LinkScheme) : null;
}

/** An address in running text: a scheme the shell opens, then everything up to a break. */
const ADDRESS = /\b(?:https?|file):\/\/[^\s<>"'`{}|\\^\p{Cc}]+/giu;

const CLOSERS: Readonly<Record<string, string>> = { ")": "(", "]": "[", "}": "{" };

/**
 * Where an address ends. Text around an address often adds punctuation that is not part
 * of it: a full stop ending the sentence, a bracket closing one opened before it.
 */
function trimAddress(address: string): string {
  let end = address.length;
  for (;;) {
    const last = address[end - 1];
    if (last === undefined) break;
    if (".,;:!?".includes(last)) {
      end--;
      continue;
    }
    const opener = CLOSERS[last];
    if (opener !== undefined) {
      const body = address.slice(0, end);
      // A closing bracket belongs to the address only if it closes one inside it.
      if (body.split(last).length > body.split(opener).length) {
        end--;
        continue;
      }
    }
    break;
  }
  return address.slice(0, end);
}

/** The address in `text` that covers the character at `index`, if there is one. */
export function addressAt(
  text: string,
  index: number,
): { start: number; end: number; target: string } | null {
  for (const match of text.matchAll(ADDRESS)) {
    const target = trimAddress(match[0]);
    const start = match.index;
    const end = start + target.length;
    if (index >= start && index < end) return { start, end, target };
    if (start > index) break;
  }
  return null;
}

/** The characters of one row, and for each UTF-16 unit the column it came from. */
interface RowText {
  text: string;
  columns: number[];
}

function rowText(frame: RenderFrame, row: number): RowText {
  let text = "";
  const columns: number[] = [];
  const first = row * frame.cols;
  for (let col = 0; col < frame.cols; col++) {
    const index = first + col;
    // The second cell of a wide character holds nothing of its own.
    if ((frame.flags[index]! & CellFlag.Spacer) !== 0) continue;
    const glyph = frame.glyph[index]!;
    const piece =
      glyph === 0
        ? " "
        : (glyph & GLYPH_CLUSTER_TAG) !== 0
          ? (frame.clusters[glyph & ~GLYPH_CLUSTER_TAG] ?? " ")
          : String.fromCodePoint(glyph);
    text += piece;
    for (let unit = 0; unit < piece.length; unit++) columns.push(col);
  }
  return { text, columns };
}

function blank(frame: RenderFrame, row: number, col: number): boolean {
  const glyph = frame.glyph[row * frame.cols + col]!;
  return glyph === 0 || glyph === 0x20;
}

/**
 * Whether the text of `row` runs on into the next row. The frame does not say which rows
 * the terminal wrapped, so this goes by appearance: a row filled to its last cell,
 * followed by one that starts with text.
 */
function continues(frame: RenderFrame, row: number): boolean {
  return row + 1 < frame.rows && !blank(frame, row, frame.cols - 1) && !blank(frame, row + 1, 0);
}

/** The address written out in the text under a cell, with the cells it covers. */
export function addressUnder(frame: RenderFrame, col: number, row: number): LinkHit | null {
  if (col < 0 || row < 0 || col >= frame.cols || row >= frame.rows) return null;
  // The second cell of a wide character stands for the character in the cell before it.
  if (col > 0 && (frame.flags[row * frame.cols + col]! & CellFlag.Spacer) !== 0) col--;

  // Join the rows the address may have wrapped across into one line.
  let top = row;
  while (top > 0 && row - top < LINK_SCAN_MAX_ROWS && continues(frame, top - 1)) top--;
  let bottom = row;
  while (bottom - row < LINK_SCAN_MAX_ROWS && continues(frame, bottom)) bottom++;

  let text = "";
  const cells: { row: number; col: number }[] = [];
  let index = -1;
  for (let r = top; r <= bottom; r++) {
    const line = rowText(frame, r);
    if (r === row) {
      const at = line.columns.findIndex((c) => c >= col);
      index = at < 0 ? -1 : text.length + at;
    }
    text += line.text;
    for (const c of line.columns) cells.push({ row: r, col: c });
  }
  if (index < 0) return null;

  const found = addressAt(text, index);
  if (!found) return null;

  const ranges: UnderlineRange[] = [];
  for (let at = found.start; at < found.end; at++) {
    const cell = cells[at]!;
    const last = ranges[ranges.length - 1];
    if (last && last.row === cell.row) last.endCol = Math.max(last.endCol, cell.col);
    else ranges.push({ row: cell.row, startCol: cell.col, endCol: cell.col });
  }
  // A wide character at the end of a run takes its second cell with it.
  for (const range of ranges) {
    const next = range.row * frame.cols + range.endCol + 1;
    if (range.endCol + 1 < frame.cols && (frame.flags[next]! & CellFlag.Spacer) !== 0) {
      range.endCol++;
    }
  }
  return { target: found.target, ranges };
}

/**
 * The cells of one row that belong to the link a program marked at `col`.
 * `targetAt` gives the target of the link on a cell of that row, or `""`.
 */
export function markedRun(
  frame: RenderFrame,
  col: number,
  row: number,
  target: string,
  targetAt: (col: number) => string,
): UnderlineRange {
  const marked = (c: number) =>
    (frame.flags[row * frame.cols + c]! & CellFlag.Hyperlink) !== 0 && targetAt(c) === target;
  let startCol = col;
  while (startCol > 0 && marked(startCol - 1)) startCol--;
  let endCol = col;
  while (endCol + 1 < frame.cols && marked(endCol + 1)) endCol++;
  return { row, startCol, endCol };
}
