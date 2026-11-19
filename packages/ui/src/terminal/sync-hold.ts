/**
 * How long a terminal's last whole screen stays on show for a program that is drawing
 * the next one.
 *
 * A program that redraws its screen may say when it begins and when it is done
 * (synchronized output). In between, what it has drawn so far is in the terminal but
 * not on show: the picture is the screen it last finished, and the two change places
 * in one frame when it says it is done. A program that does not say so in time is shown
 * as it stands, and is not waited for again until it has said it is done and begun
 * anew, so one that never ends its screen cannot leave a pane standing still.
 */

import { SYNC_HOLD_MAX_MS } from "../tuning.ts";

export class SyncHold {
  /** When the program began the screen it is drawing; 0 while it draws none. */
  #since = 0;

  /**
   * Whether the picture is to stay as it is at `now`, for a program that says it is
   * drawing (`drawing`) or does not. `began` is called when a wait begins, with how
   * long it lasts at most: nothing may come from the program to end it.
   */
  held(drawing: boolean, now: number, began?: (ms: number) => void): boolean {
    if (!drawing) {
      this.#since = 0;
      return false;
    }
    if (this.#since === 0) {
      // A time of 0 would read as no wait at all.
      this.#since = Math.max(now, Number.MIN_VALUE);
      began?.(SYNC_HOLD_MAX_MS);
    }
    return now - this.#since < SYNC_HOLD_MAX_MS;
  }
}
