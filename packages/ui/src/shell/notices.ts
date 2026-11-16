/**
 * The things the window has to tell the user that need no answer: a command that
 * could not be carried out, a configuration that was not applied, something the
 * application says.
 *
 * A notice is shown beside the user's work, not over it, and goes by itself after a
 * while. The text of a notice may come from a program or a file, so it is only ever
 * shown as text.
 */

import { NOTICE_INFO_MS, NOTICE_PROBLEM_MS, NOTICES_MAX } from "../tuning.ts";

/** How much a notice matters. */
export type NoticeLevel = "info" | "warning" | "error";

/** One notice on show. */
export interface ShownNotice {
  /** Tells the notices of one store apart. */
  readonly id: number;
  readonly level: NoticeLevel;
  readonly message: string;
}

/**
 * A message as a sentence: its first letter a capital. What the core refuses a request
 * with is worded to follow other words, and is shown here by itself.
 */
export function sentence(message: string): string {
  return message.charAt(0).toUpperCase() + message.slice(1);
}

/** No notices: the list a store holds while it shows none. */
const NONE: readonly ShownNotice[] = [];

/** Holds the notices on show, oldest first, and takes each away when its time is up. */
export class NoticeStore {
  #shown: readonly ShownNotice[] = NONE;
  #next = 1;
  readonly #timers = new Map<number, ReturnType<typeof setTimeout>>();
  readonly #listeners = new Set<() => void>();

  /** The notices on show. The same list until it changes. */
  readonly current = (): readonly ShownNotice[] => this.#shown;

  /** Calls `listener` whenever the notices on show change. Returns how to stop. */
  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  /**
   * Shows a notice. One that says what a notice on show already says only renews
   * that notice's time, so a fault that repeats does not fill the window.
   */
  push(level: NoticeLevel, message: string): void {
    const same = this.#shown.find((held) => held.level === level && held.message === message);
    if (same) {
      this.#arm(same);
      return;
    }
    const notice: ShownNotice = { id: this.#next++, level, message };
    const kept = this.#shown.slice(-(NOTICES_MAX - 1));
    for (const dropped of this.#shown) {
      if (!kept.includes(dropped)) this.#disarm(dropped.id);
    }
    this.#shown = [...kept, notice];
    this.#arm(notice);
    this.#changed();
  }

  /** Takes a notice away. */
  dismiss(id: number): void {
    if (!this.#shown.some((held) => held.id === id)) return;
    this.#disarm(id);
    const left = this.#shown.filter((held) => held.id !== id);
    this.#shown = left.length === 0 ? NONE : left;
    this.#changed();
  }

  #arm(notice: ShownNotice): void {
    this.#disarm(notice.id);
    const stays = notice.level === "info" ? NOTICE_INFO_MS : NOTICE_PROBLEM_MS;
    this.#timers.set(
      notice.id,
      setTimeout(() => this.dismiss(notice.id), stays),
    );
  }

  #disarm(id: number): void {
    const timer = this.#timers.get(id);
    if (timer !== undefined) clearTimeout(timer);
    this.#timers.delete(id);
  }

  #changed(): void {
    for (const listener of [...this.#listeners]) listener();
  }
}
