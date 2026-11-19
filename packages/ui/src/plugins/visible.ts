/**
 * Which of the plugins' things a window has on screen, as the core is told it.
 *
 * Each status item and panel of a plugin's holds its id here while it is drawn. The
 * core is told the whole set, a moment after it changes, and is told the window has
 * nothing on screen while the window is out of sight: minimised, or hidden behind
 * another on a system that says so. That is what starts a plugin that waits for one of
 * its things to be seen, and what lets the machine be read for meters only while
 * someone can see them.
 */

/** How long after a change the core is told, so that a burst of changes is told once. */
const SETTLE_MS = 120;

export class VisibleItems {
  readonly #tell: (items: readonly string[]) => void;
  readonly #counts = new Map<string, number>();
  #shown = true;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #told = "";

  /** `tell` tells the core what is on screen. */
  constructor(tell: (items: readonly string[]) => void) {
    this.#tell = tell;
  }

  /** Holds an id while its thing is drawn. Returns how to let it go. */
  hold(id: string): () => void {
    this.#counts.set(id, (this.#counts.get(id) ?? 0) + 1);
    this.#changed();
    let held = true;
    return () => {
      if (!held) return;
      held = false;
      const count = (this.#counts.get(id) ?? 1) - 1;
      if (count <= 0) this.#counts.delete(id);
      else this.#counts.set(id, count);
      this.#changed();
    };
  }

  /** Whether the window is on screen at all. */
  setShown(shown: boolean): void {
    if (this.#shown === shown) return;
    this.#shown = shown;
    this.#changed();
  }

  /** The ids on screen now, as the core is told them. */
  current(): string[] {
    return this.#shown ? [...this.#counts.keys()].sort() : [];
  }

  /** Tells the core now what is on screen, if it has changed. */
  flush(): void {
    if (this.#timer !== undefined) clearTimeout(this.#timer);
    this.#timer = undefined;
    const items = this.current();
    const written = items.join("\n");
    if (written === this.#told) return;
    this.#told = written;
    this.#tell(items);
  }

  #changed(): void {
    if (this.#timer !== undefined) return;
    this.#timer = setTimeout(() => this.flush(), SETTLE_MS);
  }

  dispose(): void {
    if (this.#timer !== undefined) clearTimeout(this.#timer);
    this.#timer = undefined;
  }
}
