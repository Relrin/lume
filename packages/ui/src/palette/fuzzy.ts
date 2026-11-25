/**
 * Finding what was meant from a few letters of it.
 *
 * A query matches a text when its letters appear in the text in order, whatever lies
 * between them. Among the ways it can match, the best is the one a person would mean:
 * letters that begin words and letters that follow one another count for more than
 * letters scattered through the middle of words. The best match's score ranks the
 * text among the others, and the places of its letters say what to mark.
 */

/** A text that matched a query. */
export interface FuzzyMatch {
  /** Higher is better. Only comparable between matches of one query. */
  readonly score: number;
  /** Where in the text each letter of the query was found, rising. */
  readonly indices: readonly number[];
}

/** What a matched letter counts for. */
const LETTER = 1;
/** What it counts for besides when it begins a word. */
const WORD_START = 8;
/** What it counts for besides when it begins the text. */
const TEXT_START = 4;
/** What it counts for besides when it follows the letter matched before it. */
const CONSECUTIVE = 6;
/** What each letter passed over between two matched ones costs, and the most a gap costs. */
const GAP = 1;
const GAP_MAX = 3;

const SEPARATORS = new Set([" ", "-", "_", ".", ":", "/", "\\"]);

/**
 * How many letters of a query are looked for, and in how much of a text. The work grows
 * with both, and neither is always the user's own: names come from elsewhere too.
 */
export const QUERY_LETTERS_MAX = 64;
export const TEXT_LETTERS_MAX = 256;

/** The first letters of a text, by character. A character takes two units at most. */
function firstLetters(text: string, max: number): string[] {
  return [...text.slice(0, 2 * max)].slice(0, max);
}

/** Whether the letter at `index` begins a word: after a separator, or a capital after a small letter. */
function beginsWord(letters: readonly string[], index: number): boolean {
  if (index === 0) return true;
  const before = letters[index - 1]!;
  if (SEPARATORS.has(before)) return true;
  const here = letters[index]!;
  return before !== before.toUpperCase() && here !== here.toLowerCase();
}

/**
 * Matches `query` against `text`, without regard to case; spaces in the query are
 * passed over. Returns nothing when the query's letters are not in the text in order.
 * An empty query matches everything, with nothing marked.
 */
export function fuzzyMatch(query: string, text: string): FuzzyMatch | undefined {
  const wanted = wantedLetters(query);
  return wanted.length === 0 ? { score: 0, indices: [] } : matchLetters(wanted, text);
}

/** The letters a query asks for, in order. */
function wantedLetters(query: string): readonly string[] {
  return firstLetters(query.toLowerCase().replace(/\s+/gu, ""), QUERY_LETTERS_MAX);
}

/** What no match scores, so low that nothing that is added to it makes it one. */
const NONE = -1_000_000;

/*
 * What matching works in, kept from one text to the next: a list of thousands of names is
 * matched at each letter typed.
 */
/** What each letter of the text counts for when it is matched, whatever comes before it. */
const worth = new Int32Array(TEXT_LETTERS_MAX);
/** The best score of the query's letters so far with the last of them matched at each place. */
let best = new Int32Array(TEXT_LETTERS_MAX);
/** The same without the query's last letter. */
let before = new Int32Array(TEXT_LETTERS_MAX);
/** For each letter of the query and each place, where the letter before it was matched. */
const from = new Int16Array(QUERY_LETTERS_MAX * TEXT_LETTERS_MAX);

function matchLetters(wanted: readonly string[], text: string): FuzzyMatch | undefined {
  const letters = firstLetters(text, TEXT_LETTERS_MAX);
  const count = letters.length;
  const lower = letters.map((letter) => letter.toLowerCase());
  // Most texts do not match, which is seen without looking for the best way to.
  let found = 0;
  for (const letter of lower) if (letter === wanted[found]) found += 1;
  if (found < wanted.length) return undefined;
  for (let j = 0; j < count; j++) {
    worth[j] = LETTER + (beginsWord(letters, j) ? WORD_START : 0) + (j === 0 ? TEXT_START : 0);
  }

  for (let j = 0; j < count; j++) {
    best[j] = lower[j] === wanted[0] ? worth[j]! - Math.min(j * GAP, GAP_MAX) : NONE;
  }
  for (let i = 1; i < wanted.length; i++) {
    const letter = wanted[i];
    const back = i * TEXT_LETTERS_MAX;
    [best, before] = [before, best];
    best.fill(NONE, 0, count);
    // The best of the places so far back that the gap to them costs the most it can.
    let far = -1;
    for (let j = i; j < count; j++) {
      const passed = j - GAP_MAX / GAP - 1;
      if (passed >= 0 && before[passed]! > (far < 0 ? NONE : before[far]!)) far = passed;
      if (lower[j] !== letter) continue;
      let at = far;
      let score = far < 0 ? NONE : before[far]! - GAP_MAX;
      for (let k = Math.max(0, passed + 1); k < j; k++) {
        const joined = k === j - 1 ? CONSECUTIVE : -(j - k - 1) * GAP;
        if (before[k]! !== NONE && before[k]! + joined > score) {
          score = before[k]! + joined;
          at = k;
        }
      }
      if (at < 0) continue;
      best[j] = score + worth[j]!;
      from[back + j] = at;
    }
  }

  let end = -1;
  for (let j = 0; j < count; j++) {
    if (best[j]! > (end < 0 ? NONE : best[end]!)) end = j;
  }
  if (end < 0) return undefined;
  const indices = new Array<number>(wanted.length);
  for (let i = wanted.length - 1, j = end; i >= 0; i--) {
    indices[i] = j;
    j = from[i * TEXT_LETTERS_MAX + j]!;
  }
  return { score: best[end]!, indices };
}

/** An item that matched, with how well and where. */
export interface Ranked<T> extends FuzzyMatch {
  readonly item: T;
}

/**
 * The items whose text matches `query`, best first. Items that match equally well
 * keep the order they were given in, so an empty query leaves the list as it was.
 */
export function rank<T>(
  items: readonly T[],
  query: string,
  textOf: (item: T) => string,
): Ranked<T>[] {
  return rankAny(items, query, (item) => [textOf(item)]);
}

/**
 * The items of which a text matches `query`, best first: each counts by the text of its
 * that matches best.
 */
export function rankAny<T>(
  items: readonly T[],
  query: string,
  textsOf: (item: T) => Iterable<string>,
): Ranked<T>[] {
  const wanted = wantedLetters(query);
  if (wanted.length === 0) return items.map((item) => ({ item, score: 0, indices: [] }));
  const matched: Ranked<T>[] = [];
  for (const item of items) {
    let best: FuzzyMatch | undefined;
    for (const text of textsOf(item)) {
      const match = matchLetters(wanted, text);
      if (match && (!best || match.score > best.score)) best = match;
    }
    if (best) matched.push({ item, ...best });
  }
  // The sort is stable.
  return matched.sort((a, b) => b.score - a.score);
}
