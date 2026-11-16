/**
 * The agreement between the terminal engine's frames and the renderer.
 *
 * `@lume/vt` produces frames and `@lume/renderer` draws them, and neither depends on the
 * other: each declares the frame's shape and the meaning of its numbers for itself. This
 * package uses both, so it is where the two declarations are held against each other.
 * The shapes are compared by the compiler, the numbers when the check runs.
 */

import * as renderer from "@lume/renderer";
import * as vt from "@lume/vt";

// A frame from the terminal must be acceptable to the renderer as it is. If the shapes
// drift apart, this line stops compiling.
const accepts: (frame: vt.RenderFrame) => renderer.RenderFrame = (frame) => frame;
void accepts;

/** The names of the numbers both packages declare. */
const SHARED = [
  "DirtyState",
  "CellFlag",
  "UnderlineStyle",
  "CursorShape",
  "UNDERLINE_SHIFT",
  "UNDERLINE_MASK",
  "GLYPH_CLUSTER_TAG",
  "NO_SELECTION",
  "HighlightKind",
  "HIGHLIGHT_STRIDE",
] as const;

/** The differences between the two packages' numbers, as text. Empty when they agree. */
export function frameContractDifferences(): string[] {
  const differences: string[] = [];
  for (const name of SHARED) {
    const ours = JSON.stringify(vt[name]);
    const theirs = JSON.stringify(renderer[name]);
    if (ours !== theirs)
      differences.push(`${name}: the terminal has ${ours}, the renderer ${theirs}`);
  }
  if (vt.rgba(1, 2, 3, 4) !== renderer.rgba(1, 2, 3, 4)) {
    differences.push("rgba packs a colour differently in the two packages");
  }
  return differences;
}

/** Throw if the terminal and the renderer disagree about what a frame's numbers mean. */
export function checkFrameContract(): void {
  const differences = frameContractDifferences();
  if (differences.length > 0) {
    throw new Error(
      `the terminal and the renderer disagree about frames:\n${differences.join("\n")}`,
    );
  }
}
