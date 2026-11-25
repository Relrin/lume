/**
 * What the terminals of servers may hold of a window, all of them together.
 *
 * A window's terminals share one engine, whose memory has an end and is never given
 * back, and each cell of a terminal on show is paid for again where it is drawn. What a
 * server has in its terminals is the server's to say: as many as it likes, as large as
 * it likes, filled as fast as they are taken. Without a bound on the whole, a server
 * could leave a window with no room to start a shell of its own in.
 *
 * The terminals of the application's own sessions are not counted: what fills those is
 * what the user runs.
 */

import {
  SERVED_CELLS_FLOOR,
  SERVED_CELLS_MAX,
  SERVED_ENGINE_MAX_BYTES,
  SERVED_SCROLLBACK_FLOOR_BYTES,
  SERVED_SCROLLBACK_MAX_BYTES,
  SERVED_SCROLLBACK_SHARE,
  SERVED_TERMINALS_MAX,
  TERMINAL_CELLS_MAX,
} from "../limits.ts";
import { withinCells, type TermSize } from "../transport/types.ts";

/** What one terminal may hold. It holds it until it is closed. */
export interface ServedShare {
  /** How much memory its scrollback may take. */
  readonly scrollbackBytes: number;
  /** The size it may have in place of `size`. It is taken to have that from now on. */
  fit(size: TermSize): TermSize;
  /** The terminal is gone. */
  close(): void;
}

/** A terminal that is open, and how many cells it has. */
interface Held {
  readonly scrollbackBytes: number;
  cells: number;
}

/** What a window's terminals of servers hold together. A window has one. */
export class ServedBudget {
  readonly #held = new Set<Held>();
  readonly #engineBytes: () => number;

  /** `engineBytes` says how much memory the window's engine holds now. */
  constructor(engineBytes: () => number) {
    this.#engineBytes = engineBytes;
  }

  /** The share of one more terminal, or nothing when there is room for none. */
  open(): ServedShare | undefined {
    if (this.#held.size >= SERVED_TERMINALS_MAX) return undefined;
    if (this.#engineBytes() > SERVED_ENGINE_MAX_BYTES) return undefined;
    let kept = 0;
    for (const other of this.#held) kept += other.scrollbackBytes;
    // A part of what is left: the first few may keep a lot, and all of them together no
    // more than there is.
    const part = Math.floor((SERVED_SCROLLBACK_MAX_BYTES - kept) / SERVED_SCROLLBACK_SHARE);
    const held: Held = {
      scrollbackBytes: Math.max(part, SERVED_SCROLLBACK_FLOOR_BYTES),
      cells: 0,
    };
    this.#held.add(held);
    return {
      scrollbackBytes: held.scrollbackBytes,
      fit: (size) => {
        let others = 0;
        for (const other of this.#held) if (other !== held) others += other.cells;
        const left = Math.max(SERVED_CELLS_MAX - others, SERVED_CELLS_FLOOR);
        const fitted = withinCells(size, Math.min(left, TERMINAL_CELLS_MAX));
        held.cells = fitted.cols * fitted.rows;
        return fitted;
      },
      close: () => {
        this.#held.delete(held);
      },
    };
  }
}
