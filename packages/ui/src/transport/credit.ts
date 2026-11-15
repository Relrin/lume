/**
 * Flow control on the receiving side of one stream.
 *
 * A session sends no more output than it has been granted. The page grants one window when
 * it attaches and tops it up as output is consumed, so at most one window is ever in
 * flight or waiting, and a page that stops consuming stops the program.
 */

/** The core sent more output than it had been granted. */
export class CreditError extends Error {
  constructor(granted: number, received: number) {
    super(`${received} bytes arrived with ${granted} granted`);
    this.name = "CreditError";
  }
}

/** Decides when to grant more credit for one stream. */
export class Granter {
  readonly #window: number;
  #outstanding: number;
  #consumed = 0;

  /**
   * Starts a stream with `window` bytes of credit. The caller sends that first grant
   * itself: see {@link firstGrant}.
   */
  constructor(window: number) {
    if (!Number.isInteger(window) || window < 1) {
      throw new RangeError(`a credit window of ${window} bytes`);
    }
    this.#window = window;
    this.#outstanding = window;
  }

  /** The grant that opens the stream. */
  get firstGrant(): number {
    return this.#window;
  }

  /** Credit granted and not yet used by the sender. */
  get outstanding(): number {
    return this.#outstanding;
  }

  /** Records `bytes` of output received. Throws if the sender exceeded its credit. */
  received(bytes: number): void {
    if (bytes > this.#outstanding) throw new CreditError(this.#outstanding, bytes);
    this.#outstanding -= bytes;
  }

  /**
   * Records that `bytes` of received output have been processed. Returns a grant to send
   * once half a window has been freed, and zero otherwise. Granting in halves keeps the
   * sender supplied without a credit frame for every frame of output.
   */
  consumed(bytes: number): number {
    this.#consumed = Math.min(this.#consumed + bytes, this.#window);
    if (this.#consumed < Math.ceil(this.#window / 2)) return 0;
    const grant = this.#consumed;
    this.#consumed = 0;
    this.#outstanding += grant;
    return grant;
  }
}
