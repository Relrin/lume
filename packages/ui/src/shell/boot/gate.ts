/**
 * Between the core and a page that is still starting.
 *
 * The core may speak as soon as the carrier is open, before the page has its first
 * copies to follow changes against. What it says meanwhile is kept, and followed in
 * the order it came in once the page goes live; a change the copies already hold is
 * passed over by whatever follows it. What the core asks is held until then too.
 */

import type { Notice } from "../../core/types.ts";

export class CoreGate {
  #early: Notice[] = [];
  #held: unknown[] = [];
  #follow: ((notice: Notice) => void) | undefined;
  #answer: ((message: unknown) => void) | undefined;
  #live = false;
  #closed = false;

  /** What to hand `CoreClient.open` for the core's notices and requests. */
  readonly handlers = {
    notice: (notice: Notice): void => this.notice(notice),
    request: (message: unknown): void => this.request(message),
  };

  /** Whether the page has gone live. */
  get live(): boolean {
    return this.#live;
  }

  /** A notice from the core: followed if the page is live, kept if not. */
  notice(notice: Notice): void {
    if (this.#closed) return;
    if (this.#live) this.#follow?.(notice);
    else this.#early.push(notice);
  }

  /** A request from the core: answered if the page is live, held if not. */
  request(message: unknown): void {
    if (this.#closed) return;
    if (this.#live) this.#answer?.(message);
    else this.#held.push(message);
  }

  /**
   * The page has its copies and can answer: what was kept is followed, then what was
   * held is answered, each in the order it came. A page that answers nothing passes
   * no `answer`, and what the core asks is let go.
   */
  goLive(follow: (notice: Notice) => void, answer?: (message: unknown) => void): void {
    if (this.#closed || this.#live) return;
    this.#live = true;
    this.#follow = follow;
    this.#answer = answer;
    const early = this.#early;
    const held = this.#held;
    this.#early = [];
    this.#held = [];
    for (const notice of early) follow(notice);
    if (answer) for (const message of held) answer(message);
  }

  /** The page has ended: nothing more is followed or answered. */
  close(): void {
    this.#closed = true;
    this.#follow = undefined;
    this.#answer = undefined;
    this.#early = [];
    this.#held = [];
  }
}
