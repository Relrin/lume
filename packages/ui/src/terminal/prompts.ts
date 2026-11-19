/**
 * The questions and notices a terminal puts to the user.
 *
 * The terminal itself is drawn outside React. The few things that need ordinary interface
 * (confirming a paste, a link or a close, a notice) are kept here as plain data, and a
 * small component shows whichever is current. The text in them comes from the clipboard
 * or from a program's output, so it is only ever shown as text.
 *
 * Plugins put questions to the user through the core: a permission they ask for, a use
 * of one the user approves each time, a question of their own, a choice from a list.
 * Each is shown under the plugin's name, and what the plugin wrote is shown as text.
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

/** The plugin a question comes from. */
export interface PluginAsker {
  readonly id: string;
  readonly name: string;
}

/** A question a plugin puts to the user, answered yes or no. */
export type PluginQuestion =
  | {
      /** A permission the plugin asks for while it runs. */
      readonly kind: "permission";
      readonly plugin: PluginAsker;
      readonly permission: string;
      /** What the permission lets the plugin do, in words. */
      readonly what: string;
      readonly warning?: string | null;
      /** Why the plugin says it asks. */
      readonly reason: string;
    }
  | {
      /** One use of a permission the user approves each time. */
      readonly kind: "each";
      readonly plugin: PluginAsker;
      readonly permission: string;
      /** The command that would run, or the text that would be typed, whole. */
      readonly text: string;
      /** Where the text would be typed. */
      readonly place?: string | null;
    }
  | {
      readonly kind: "confirm";
      readonly plugin: PluginAsker;
      readonly title: string;
      readonly message: string;
      readonly confirm: string;
      readonly danger: boolean;
    };

/** One choice of a plugin's list. */
export interface PickItem {
  readonly id: string;
  readonly label: string;
  readonly detail?: string | null;
}

/** A choice a plugin asks the user to make from a list. */
export interface PluginPick {
  readonly kind: "pick";
  readonly plugin: PluginAsker;
  readonly title?: string | null;
  readonly placeholder?: string | null;
  readonly items: readonly PickItem[];
}

/** Anything a plugin may put to the user. */
export type PluginAsk = PluginQuestion | PluginPick;

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
  | { readonly kind: "notice"; readonly message: string; answer(ok: boolean): void }
  | { readonly kind: "plugin"; readonly ask: PluginQuestion; answer(ok: boolean): void }
  | {
      readonly kind: "pick";
      readonly ask: PluginPick;
      /** The id chosen, or none for a choice put away. */
      answer(ok: boolean, chosen?: string): void;
    };

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

  /**
   * Puts a plugin's question to the user, and gives the answer as the core takes it:
   * `{ allow }` for a permission or a use, `{ ok }` for a question, `{ chosen }` for a
   * choice.
   */
  async askPlugin(ask: PluginAsk): Promise<Record<string, unknown>> {
    if (ask.kind === "pick") {
      let chosen: string | null = null;
      await this.#ask((answer) => ({
        kind: "pick",
        ask,
        answer: (ok, id) => {
          chosen = ok ? (id ?? null) : null;
          answer(ok);
        },
      }));
      return { chosen };
    }
    const ok = await this.#ask((answer) => ({ kind: "plugin", ask, answer }));
    return ask.kind === "confirm" ? { ok } : { allow: ok };
  }

  /** Answers every question waiting with its cautious answer, as when nobody is left to ask. */
  dismissAll(): void {
    for (const prompt of [...this.#queue]) prompt.answer(false);
  }

  /**
   * Answers the question on show as the user's keys would: going ahead (with the
   * choice `item`, for a choice), or not. Says whether there was one.
   */
  answerCurrent(proceed: boolean, item?: string): boolean {
    const current = this.current();
    if (!current) return false;
    if (current.kind === "pick") {
      const known = item !== undefined && current.ask.items.some((one) => one.id === item);
      current.answer(proceed && known, known ? item : undefined);
    } else {
      current.answer(proceed);
    }
    return true;
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
