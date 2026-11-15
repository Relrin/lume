/**
 * From a key event to a command.
 *
 * A key is read as a chord, the chord is looked up in the bindings in effect where the
 * key was pressed, and a command it is bound to is run. The caller learns only whether
 * the shell took the key, which is what decides whether the program in a terminal gets
 * it.
 */

import type { KeyBinding } from "../core/types.ts";
import type { KeyEventLike } from "../terminal/keyboard.ts";
import { chordOf, type ChordMapping } from "./chord.ts";
import { KeyResolver } from "./resolver.ts";

/**
 * The commands a key that is held down runs again and again: moving about and
 * scrolling. Any other command runs once for each press, so holding the key for a new
 * tab a moment too long does not open ten.
 */
const REPEATABLE = [
  "lume.pane.focus-",
  "lume.tab.next",
  "lume.tab.previous",
  "lume.terminal.scroll-",
  "lume.view.zoom-",
];

/** Turns keys into commands under the bindings it was last given. */
export class Shortcuts {
  readonly #resolver = new KeyResolver();
  readonly #mapping: ChordMapping;
  readonly #run: (command: string, args: unknown) => void;

  constructor(mapping: ChordMapping, run: (command: string, args: unknown) => void) {
    this.#mapping = mapping;
    this.#run = run;
  }

  /** Replaces the bindings: the configuration changed. */
  setBindings(bindings: readonly KeyBinding[]): void {
    this.#resolver.setBindings(bindings);
  }

  /**
   * Offers a key to the bindings that hold in `contexts`. Returns whether the shell
   * took it: it ran a command, or the key may be the beginning of a sequence that does.
   */
  handle(event: KeyEventLike, contexts: ReadonlySet<string>): boolean {
    const chord = chordOf(event, this.#mapping);
    if (chord === null) return false;
    const outcome = this.#resolver.resolve(chord, contexts, performance.now());
    if (outcome.kind === "pass") return false;
    if (outcome.kind === "run") {
      const { command } = outcome;
      if (!event.repeat || REPEATABLE.some((prefix) => command.startsWith(prefix))) {
        this.#run(command, outcome.args);
      }
    }
    return true;
  }
}
