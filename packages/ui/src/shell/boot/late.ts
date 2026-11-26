/**
 * What the page makes further on, for what is called back before it is.
 *
 * Some of the page's parts need each other: the views run commands, and the commands
 * need the views. One of the two is handed a cell that the other is put in once it is
 * made. A cell read before then is a mistake in the order of the start, and says so.
 */

/** A thing that is made later in the start than where it is first handed out. */
export interface Late<T> {
  /** Puts the thing in, once. */
  set(value: T): void;
  /** The thing. Throws if the start has not made it yet. */
  get(): T;
  /** The thing, or nothing while it is not made yet. */
  peek(): T | undefined;
  /** Runs `use` with the thing: now if it is made, else as soon as it is. */
  whenSet(use: (value: T) => void): void;
}

/** A cell for the thing `name` names, empty until it is set. */
export function late<T>(name: string): Late<T> {
  let made = false;
  let value: T | undefined;
  let waiting: ((value: T) => void)[] = [];
  return {
    set(next) {
      if (made) throw new Error(`${name} was set twice`);
      made = true;
      value = next;
      const queued = waiting;
      waiting = [];
      for (const use of queued) use(next);
    },
    get() {
      if (!made) throw new Error(`${name} used before boot reached it`);
      return value as T;
    },
    peek: () => value,
    whenSet(use) {
      if (made) use(value as T);
      else waiting.push(use);
    },
  };
}
