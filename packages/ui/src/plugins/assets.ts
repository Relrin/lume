/**
 * The plugins' pictures and the shaders of their cursor effects, fetched when needed.
 *
 * The core names each by the hash of its bytes and keeps the bytes to itself: a window
 * asks for a picture when it first draws it and for a shader when its effect is the
 * one chosen, so a window that shows neither never holds them. What was fetched is kept
 * by hash, which says what it is whatever else changes, so the same picture is asked
 * for once however often what plugins add is worked out again.
 *
 * Everything asked for in one turn of the page goes to the core as one question.
 */

/** Asks the core for pictures and shaders by their hashes; those it has come back. */
export type FetchAssets = (hashes: readonly string[]) => Promise<Readonly<Record<string, string>>>;

export class Assets {
  readonly #bodies = new Map<string, string>();
  /** Asked for and not yet answered, by hash, with who waits for the answer. */
  readonly #asked = new Map<string, Promise<void>>();
  /** Hashes the core did not have: not asked for again until something changes. */
  readonly #missing = new Set<string>();
  readonly #listeners = new Set<() => void>();
  #queued: string[] = [];
  #flush: Promise<void> | undefined;
  #fetch: FetchAssets | undefined;

  /** Says how the core is asked. Until then nothing is fetched. Gives how to stop. */
  connect(fetch: FetchAssets): () => void {
    this.#fetch = fetch;
    return () => {
      if (this.#fetch === fetch) this.#fetch = undefined;
    };
  }

  /** What was fetched under a hash: a picture's address or a shader's text. */
  peek(hash: string): string | undefined {
    return this.#bodies.get(hash);
  }

  /** Makes sure a hash is fetched, if the core has it. */
  want(hash: string): void {
    void this.load([hash]);
  }

  /** Fetches the hashes that are not held yet. Settles when all were answered. */
  load(hashes: readonly string[]): Promise<void> {
    const waits: Promise<void>[] = [];
    for (const hash of hashes) {
      if (this.#bodies.has(hash) || this.#missing.has(hash)) continue;
      const asked = this.#asked.get(hash);
      if (asked) {
        waits.push(asked);
        continue;
      }
      if (!this.#fetch) continue;
      this.#queued.push(hash);
      const wait = this.#soon();
      this.#asked.set(hash, wait);
      waits.push(wait);
    }
    return Promise.all(waits).then(() => undefined);
  }

  /** What plugins add was worked out again: a hash the core lacked may be there now. */
  retry(): void {
    this.#missing.clear();
  }

  /** Holds a body without asking, as a test or a stand-in for the core does. */
  put(hash: string, body: string): void {
    this.#bodies.set(hash, body);
    this.#tell();
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  /** One question for everything asked for in this turn of the page. */
  #soon(): Promise<void> {
    this.#flush ??= Promise.resolve().then(async () => {
      const hashes = this.#queued;
      this.#queued = [];
      this.#flush = undefined;
      let found: Readonly<Record<string, string>>;
      try {
        found = (await this.#fetch?.(hashes)) ?? {};
      } catch {
        // Asked for again the next time one of them is wanted.
        for (const hash of hashes) this.#asked.delete(hash);
        return;
      }
      for (const hash of hashes) {
        this.#asked.delete(hash);
        const body = found[hash];
        if (typeof body === "string") this.#bodies.set(hash, body);
        else this.#missing.add(hash);
      }
      this.#tell();
    });
    return this.#flush;
  }

  #tell(): void {
    for (const listener of [...this.#listeners]) listener();
  }
}

/** The window's pictures and shaders of plugins. */
export const assets = new Assets();
