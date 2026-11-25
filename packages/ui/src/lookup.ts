/**
 * The entry of a table for a word that came from outside the page. A table is an object
 * like any other: it answers to `constructor` and the like with something that is no
 * entry of it.
 */
export function lookup<T>(table: Readonly<Record<string, T>>, word: string): T | undefined {
  return Object.hasOwn(table, word) ? table[word] : undefined;
}
