/**
 * Which command a key runs.
 *
 * The core sends the bindings in effect: each a sequence of chords, a command, and the
 * contexts it holds in. The resolver is given one chord at a time, as text, with the
 * contexts that hold where the key was pressed, and says whether the chord runs a
 * command, is the beginning of a sequence, or is none of the shell's business and goes
 * on to the program.
 *
 * It keeps no clock of its own: whoever calls it says what time it is, so the rules
 * for sequences can be tested without waiting.
 */

import type { KeyBinding } from "../core/types.ts";
import { KEY_SEQUENCE_TIMEOUT_MS } from "../tuning.ts";

/** What a chord came to. */
export type KeyOutcome =
  /** The chord, with those before it, is bound: run the command. */
  | { readonly kind: "run"; readonly command: string; readonly args: unknown }
  /** The chord begins a sequence: keep it from the program and wait for the next. */
  | { readonly kind: "pending" }
  /** The chord is not the shell's: it goes on to the program. */
  | { readonly kind: "pass" };

const PASS: KeyOutcome = { kind: "pass" };
const PENDING: KeyOutcome = { kind: "pending" };

/** One condition on where a binding holds. */
interface Term {
  readonly name: string;
  readonly negated: boolean;
}

/** A binding, read once. */
interface Bound {
  readonly keys: readonly string[];
  readonly command: string;
  readonly args: unknown;
  readonly when: readonly Term[];
  /** Its place among the bindings: a later one wins over an earlier one. */
  readonly order: number;
}

function holds(when: readonly Term[], contexts: ReadonlySet<string>): boolean {
  return when.every((term) => contexts.has(term.name) !== term.negated);
}

function startsWith(keys: readonly string[], chords: readonly string[]): boolean {
  return keys.length >= chords.length && chords.every((chord, index) => keys[index] === chord);
}

/** Resolves chords to commands under the bindings it was last given. */
export class KeyResolver {
  #bindings: readonly Bound[] = [];
  /** The chords of a sequence pressed so far. */
  #prefix: string[] = [];
  /** When the last of them was pressed. */
  #since = 0;

  /**
   * Replaces the bindings. They are taken in the order the core sends them: the
   * defaults first, then the user's, so that of two bindings that are otherwise equal
   * the later is the user's and wins. A sequence in progress is forgotten.
   */
  setBindings(bindings: readonly KeyBinding[]): void {
    this.#bindings = bindings
      .filter((binding) => binding.keys.length > 0)
      .map((binding, order) => ({
        keys: binding.keys,
        command: binding.command,
        args: binding.args,
        when: binding.when.map((term) =>
          term.startsWith("!")
            ? { name: term.slice(1), negated: true }
            : { name: term, negated: false },
        ),
        order,
      }));
    this.reset();
  }

  /** Forgets a sequence in progress: the next chord begins afresh. */
  reset(): void {
    this.#prefix = [];
  }

  /** Whether the first chords of a sequence have been pressed and the rest is awaited. */
  pending(nowMs: number): boolean {
    return this.#prefix.length > 0 && nowMs - this.#since <= KEY_SEQUENCE_TIMEOUT_MS;
  }

  /**
   * What `chord` comes to, pressed at `nowMs` where `contexts` hold.
   *
   * A binding holds when each of its contexts is among `contexts`, and each it negates
   * is not. Of the bindings that hold and that the chords pressed so far complete, the
   * one with more contexts wins, and of those the later one. A chord that completes a
   * binding runs it at once, even if a longer sequence begins the same way: a sequence
   * is reached only through chords that are bound to nothing by themselves. A chord
   * that only begins sequences is kept and waited on, for a limited time. A chord that
   * follows the beginning of a sequence and completes none is passed on, and the
   * beginning is forgotten.
   */
  resolve(chord: string, contexts: ReadonlySet<string>, nowMs: number): KeyOutcome {
    if (this.#prefix.length > 0 && nowMs - this.#since > KEY_SEQUENCE_TIMEOUT_MS) this.reset();
    const pressed = [...this.#prefix, chord];

    let complete: Bound | undefined;
    let longer = false;
    for (const binding of this.#bindings) {
      if (!startsWith(binding.keys, pressed) || !holds(binding.when, contexts)) continue;
      if (binding.keys.length > pressed.length) {
        longer = true;
      } else if (
        complete === undefined ||
        binding.when.length > complete.when.length ||
        (binding.when.length === complete.when.length && binding.order > complete.order)
      ) {
        complete = binding;
      }
    }

    if (complete) {
      this.reset();
      return { kind: "run", command: complete.command, args: complete.args };
    }
    if (longer) {
      this.#prefix = pressed;
      this.#since = nowMs;
      return PENDING;
    }
    this.reset();
    return PASS;
  }
}
