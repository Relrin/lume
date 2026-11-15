/**
 * Result codes of libghostty-vt and the error thrown for the unexpected ones.
 */

// Values of `GhosttyResult` from the C headers of the pinned release (ghostty/vt/types.h).
// They are part of the stable ABI, and they are needed before the type manifest can be
// read, so they are the one set of numbers written out here.
export const SUCCESS = 0;
export const OUT_OF_MEMORY = -1;
export const OUT_OF_SPACE = -3;
export const NO_VALUE = -4;
export const UNSUPPORTED = -8;

/** A libghostty-vt call failed. */
export class VtError extends Error {
  constructor(
    /** The `GhosttyResult` the call returned. */
    readonly code: number,
    what: string,
  ) {
    super(`libghostty-vt: ${what} failed with code ${code}`);
    this.name = "VtError";
  }
}
