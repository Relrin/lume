/**
 * The questions and notices a terminal puts to the user.
 *
 * The terminal itself is drawn outside React. The few things that need ordinary interface
 * (confirming a paste, a link or a close, a notice) are kept here as plain data, and a
 * small component shows whichever is current. The text in them comes from the clipboard
 * or from a program's output, so it is only ever shown as text.
 */

import type { PastePreview } from "./clipboard.ts";

/** Something about to be closed that still has programs running in it. */
export interface Closing {
  /**
   * What is being closed: a pane, a tab, several tabs, a group of tabs, or a window;
   * or a group that is moving to another workspace, which ends its programs too.
   */
  readonly what: "pane" | "tab" | "tabs" | "group" | "moved-group" | "window";
  /** How many of its panes have a program that has not ended. At least one. */
  readonly running: number;
}

/** A question about something that cannot be taken back, put by the core. */
export interface Confirmation {
  readonly title: string;
  readonly message: string;
  /** The label of the button that goes ahead. */
  readonly confirm: string;
}

/** One thing waiting for the user. */
export type Prompt =
  | { readonly kind: "paste"; readonly preview: PastePreview; answer(ok: boolean): void }
  | { readonly kind: "link"; readonly target: string; answer(ok: boolean): void }
  | { readonly kind: "close"; readonly closing: Closing; answer(ok: boolean): void }
  | {
      readonly kind: "confirm";
      readonly confirmation: Confirmation;
      answer(ok: boolean): void;
    }
  | { readonly kind: "notice"; readonly message: string; answer(ok: boolean): void };

/** Holds the prompts in the order they were raised and shows them one at a time. */
export class PromptStore {
  #queue: Prompt[] = [];
  readonly #listeners = new Set<() => void>();

  /** The prompt to show, or `null`. The same object until it is answered. */
  readonly current = (): Prompt | null => this.#queue[0] ?? null;

  /** Calls `listener` whenever the current prompt changes. Returns how to stop. */
  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  /** Asks whether to go ahead with a paste. */
  confirmPaste(preview: PastePreview): Promise<boolean> {
    return this.#ask((answer) => ({ kind: "paste", preview, answer }));
  }

  /** Asks whether to open a link, showing where it leads. */
  confirmLink(target: string): Promise<boolean> {
    return this.#ask((answer) => ({ kind: "link", target, answer }));
  }

  /** Asks whether to close something that would end the programs running in it. */
  confirmClose(closing: Closing): Promise<boolean> {
    return this.#ask((answer) => ({ kind: "close", closing, answer }));
  }

  /** Asks whether to go ahead with something that cannot be taken back. */
  confirm(confirmation: Confirmation): Promise<boolean> {
    return this.#ask((answer) => ({ kind: "confirm", confirmation, answer }));
  }

  /** Tells the user something. It stays until dismissed. */
  notice(message: string): void {
    void this.#ask((answer) => ({ kind: "notice", message, answer }));
  }

  #ask(make: (answer: (ok: boolean) => void) => Prompt): Promise<boolean> {
    return new Promise((resolve) => {
      const prompt = make((ok) => {
        const at = this.#queue.indexOf(prompt);
        if (at < 0) return;
        this.#queue = this.#queue.filter((queued) => queued !== prompt);
        resolve(ok);
        this.#changed();
      });
      this.#queue = [...this.#queue, prompt];
      this.#changed();
    });
  }

  #changed(): void {
    for (const listener of this.#listeners) listener();
  }
}
