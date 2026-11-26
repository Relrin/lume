/**
 * What the page undoes when it ends, in the reverse of the order it was done in.
 *
 * Each step of the start adds what undoes it as it goes: what was made last goes
 * first, so nothing is left listening to something already gone.
 */

export class Disposer {
  #undo: (() => void)[] = [];
  #disposed = false;

  /** Adds a step that undoes something. */
  add(undo: () => void): void {
    this.#undo.push(undo);
  }

  /** Adds `thing`'s own `dispose`, and gives the thing back. */
  use<T extends { dispose(): void }>(thing: T): T {
    this.add(() => thing.dispose());
    return thing;
  }

  /**
   * Undoes everything added, last first; once only. A step that throws does not keep
   * the others from running: the first error is thrown once all have run.
   */
  readonly dispose = (): void => {
    if (this.#disposed) return;
    this.#disposed = true;
    let failure: { readonly error: unknown } | undefined;
    for (let undo = this.#undo.pop(); undo; undo = this.#undo.pop()) {
      try {
        undo();
      } catch (error) {
        failure ??= { error };
      }
    }
    if (failure) throw failure.error;
  };
}
