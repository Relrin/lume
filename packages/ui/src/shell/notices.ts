/**
 * The things the window has to tell the user that need no answer: a command that
 * could not be carried out, a configuration that was not applied, something the
 * application says.
 *
 * A notice is shown beside the user's work, not over it, and goes by itself after a
 * while. The text of a notice may come from a program or a file, so it is only ever
 * shown as text.
 *
 * What shows the notices is the page's toaster (see `Notices.tsx`), which keeps them
 * in a stack and takes each away when its time is up. This holds only what was said
 * before there was a toaster to show it, and names each notice by what it says, so a
 * fault that repeats is one notice and not a window full of them. Nothing here touches
 * the page, so it is tested with plain functions.
 */

import type { NoticeAction } from "../core/types.ts";
import { NOTICE_INFO_MS, NOTICE_PROBLEM_MS, NOTICES_MAX } from "../tuning.ts";

/** How much a notice matters. */
export type NoticeLevel = "info" | "warning" | "error";

/** One notice, as it is handed over to be shown. */
export interface ShownNotice {
  /**
   * Tells the notices of a page apart by what they say: a notice shown again under the
   * key of one on show takes that one's place.
   */
  readonly key: string;
  readonly level: NoticeLevel;
  readonly message: string;
  /** What the user may do about it, as a button beside the message. */
  readonly action?: NoticeAction;
  /** How long it stays before it goes by itself, in milliseconds. */
  readonly stays: number;
}

/** What shows the notices: the page's toaster. */
export interface NoticeSurface {
  show(notice: ShownNotice): void;
  hide(key: string): void;
}

/**
 * A message as a sentence: its first letter a capital. What the core refuses a request
 * with is worded to follow other words, and is shown here by itself.
 */
export function sentence(message: string): string {
  return message.charAt(0).toUpperCase() + message.slice(1);
}

/** The key of a notice: what tells it from one that says something else. */
export function noticeKey(level: NoticeLevel, message: string, action?: NoticeAction): string {
  return [level, action?.command ?? "", message].join("\u0000");
}

/** Hands the page's notices to what shows them, keeping the early ones until it can. */
export class NoticeStore {
  #surface: NoticeSurface | null = null;
  /** What was said before anything could show it, oldest first. */
  #waiting: ShownNotice[] = [];

  /**
   * Shows a notice. One that says what a notice on show already says takes that
   * notice's place, so a fault that repeats does not fill the window.
   */
  push(level: NoticeLevel, message: string, action?: NoticeAction): void {
    const key = noticeKey(level, message, action);
    const stays = level === "info" ? NOTICE_INFO_MS : NOTICE_PROBLEM_MS;
    const notice: ShownNotice =
      action === undefined
        ? { key, level, message, stays }
        : { key, level, message, action, stays };
    if (this.#surface) {
      this.#surface.show(notice);
      return;
    }
    const others = this.#waiting.filter((held) => held.key !== key);
    this.#waiting = [...others, notice].slice(-NOTICES_MAX);
  }

  /** Takes a notice away, by its key. */
  dismiss(key: string): void {
    this.#waiting = this.#waiting.filter((held) => held.key !== key);
    this.#surface?.hide(key);
  }

  /**
   * Says what shows the notices from now on, and hands it the ones that waited.
   * Returns how to take it away again.
   */
  connect(surface: NoticeSurface): () => void {
    this.#surface = surface;
    const waited = this.#waiting;
    this.#waiting = [];
    for (const notice of waited) surface.show(notice);
    return () => {
      if (this.#surface === surface) this.#surface = null;
    };
  }
}
