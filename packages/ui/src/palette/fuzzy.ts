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

/** Whether the letter at `index` begins a word: after a separator, or a capital after a small letter. */
function beginsWord(text: string, index: number): boolean {
  if (index === 0) return true;
  const before = text[index - 1]!;
  if (SEPARATORS.has(before)) return true;
  const here = text[index]!;
  return before !== before.toUpperCase() && here !== here.toLowerCase();
}

/**
 * Matches `query` against `text`, without regard to case; spaces in the query are
 * passed over. Returns nothing when the query's letters are not in the text in order.
 * An empty query matches everything, with nothing marked.
 */
export function fuzzyMatch(query: string, text: string): FuzzyMatch | undefined {
  const wanted = [...query.toLowerCase()].filter((letter) => letter.trim() !== "");
  if (wanted.length === 0) return { score: 0, indices: [] };
  const letters = [...text];
  const lower = letters.map((letter) => letter.toLowerCase());
  const plain = letters.join("");
  if (wanted.length > letters.length) return undefined;

  // best[i][j]: the best score of the query's first i + 1 letters with the last of
  // them matched at j. from[i][j]: where the letter before it was matched.
  const best: number[][] = [];
  const from: number[][] = [];
  for (let i = 0; i < wanted.length; i++) {
    const row = new Array<number>(letters.length).fill(-Infinity);
    const back = new Array<number>(letters.length).fill(-1);
    for (let j = i; j < letters.length; j++) {
      if (lower[j] !== wanted[i]) continue;
      let worth = LETTER;
      if (beginsWord(plain, j)) worth += WORD_START;
      if (j === 0) worth += TEXT_START;
      if (i === 0) {
        row[j] = worth - Math.min(j * GAP, GAP_MAX);
        continue;
      }
      const before = best[i - 1]!;
      for (let k = i - 1; k < j; k++) {
        const prior = before[k]!;
        if (prior === -Infinity) continue;
        const joined = k === j - 1 ? CONSECUTIVE : -Math.min((j - k - 1) * GAP, GAP_MAX);
        if (prior + joined + worth > row[j]!) {
          row[j] = prior + joined + worth;
          back[j] = k;
        }
      }
    }
    best.push(row);
    from.push(back);
  }

  const last = best[wanted.length - 1]!;
  let end = -1;
  for (let j = 0; j < last.length; j++) {
    if (last[j]! > (end < 0 ? -Infinity : last[end]!)) end = j;
  }
  if (end < 0) return undefined;
  const indices: number[] = [];
  for (let i = wanted.length - 1, j = end; i >= 0; i--) {
    indices.unshift(j);
    j = from[i]![j]!;
  }
  return { score: last[end]!, indices };
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
  const matched: Ranked<T>[] = [];
  for (const item of items) {
    const match = fuzzyMatch(query, textOf(item));
    if (match) matched.push({ item, ...match });
  }
  // The sort is stable.
  return matched.sort((a, b) => b.score - a.score);
}
