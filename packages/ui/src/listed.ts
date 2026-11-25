/**
 * Lists that may be longer than anyone reads: how much of one is put on the page, and
 * what is said of the rest.
 */

/** What is listed of a longer list. */
export interface Listed<T> {
  readonly rows: readonly T[];
  /** How many there are besides. */
  readonly more: number;
}

/**
 * The first `max` of a list. The first of it that is `kept` is listed whatever comes
 * before it: the last of those listed gives way to it.
 */
export function listed<T>(
  all: readonly T[],
  max: number,
  kept: (item: T) => boolean = () => false,
): Listed<T> {
  if (all.length <= max) return { rows: all, more: 0 };
  const rows = all.slice(0, max);
  if (!rows.some(kept)) {
    const late = all.find(kept);
    if (late !== undefined) rows.splice(max - 1, 1, late);
  }
  return { rows, more: all.length - rows.length };
}

/** What is said of those that are not listed. */
export function moreLine(more: number): string {
  return `and ${more} more`;
}
