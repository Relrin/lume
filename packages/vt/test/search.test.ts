import { beforeAll, describe, expect, it } from "vitest";

import {
  HIGHLIGHT_STRIDE,
  HighlightKind,
  newHighlights,
  Search,
  type Highlights,
  type Terminal,
  type Vt,
} from "../src/index.ts";
import { loadVt } from "./support.ts";

let vt: Vt;
beforeAll(async () => {
  vt = await loadVt();
});

const ESC = "\x1b";

/** Room for every line these tests write. */
const SCROLLBACK_BYTES = 32 * 1024 * 1024;

/** Longer than any search in these tests takes. */
const LONG_MS = 60_000;

function withSearch(
  cols: number,
  rows: number,
  body: (term: Terminal, search: Search) => void,
  scrollbackMaxLines = 1000,
): void {
  const term = vt.createTerminal(cols, rows, {
    scrollbackMaxLines,
    scrollbackMaxBytes: SCROLLBACK_BYTES,
  });
  const search = new Search(term);
  try {
    body(term, search);
  } finally {
    search.free();
    term.free();
  }
}

/**
 * Search until everything the terminal holds now has been looked at. A search that had
 * finished is told of what was written since by the feed.
 */
function finish(search: Search): void {
  search.feed();
  expect(search.pump(LONG_MS, Number.MAX_SAFE_INTEGER)).toBe("complete");
}

/** The runs of a list of highlights, each as row, first column, last column and kind. */
function runs(highlights: Highlights): number[][] {
  const out: number[][] = [];
  for (let index = 0; index < highlights.count; index++) {
    const at = index * HIGHLIGHT_STRIDE;
    out.push(Array.from(highlights.data.subarray(at, at + HIGHLIGHT_STRIDE)));
  }
  return out.sort((a, b) => a[0]! - b[0]! || a[1]! - b[1]!);
}

/** The runs in the viewport as the search knows them after one more feed. */
function shown(search: Search): number[][] {
  const highlights = newHighlights();
  search.feed();
  search.readViewport(highlights);
  return runs(highlights);
}

/** `count` numbered lines, each holding the word once. */
function lines(count: number, word = "needle"): string {
  let text = "";
  for (let line = 0; line < count; line++) text += `${line} ${word}\r\n`;
  return text;
}

describe("searching a terminal", () => {
  it("finds text on the screen and in what has scrolled off it", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(50));
      expect(search.total()).toBe(0);
      search.setNeedle("needle");
      expect(search.status()).toBe("feed-required");
      finish(search);
      expect(search.total()).toBe(50);
    });
  });

  it("matches letters in either case and nothing else loosely", () => {
    withSearch(20, 5, (term, search) => {
      term.write("Needle NEEDLE needle néédle");
      search.setNeedle("nEEDLe");
      finish(search);
      expect(search.total()).toBe(3);
    });
  });

  it("has nothing to do without a text to look for", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(10));
      expect(search.status()).toBe("complete");
      expect(search.pump(LONG_MS, Number.MAX_SAFE_INTEGER)).toBe("complete");
      expect(search.total()).toBe(0);
      expect(search.next()).toBe(false);
      expect(search.selectedIndex()).toBeUndefined();
      expect(search.selectedRow()).toBeUndefined();
    });
  });

  it("keeps what it found for the same text, starts again for another, and ends for none", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(30));
      search.setNeedle("needle");
      finish(search);
      search.setNeedle("NEEDLE");
      expect(search.total()).toBe(30);
      expect(search.status()).toBe("complete");

      search.setNeedle("7 needle");
      expect(search.status()).toBe("feed-required");
      finish(search);
      expect(search.total()).toBe(3);

      search.setNeedle("");
      expect(search.status()).toBe("complete");
      expect(search.total()).toBe(0);
    });
  });

  it("sees what was written after it began at the next feed", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(10));
      search.setNeedle("needle");
      finish(search);
      expect(search.total()).toBe(10);
      term.write(lines(5));
      expect(search.total()).toBe(10);
      finish(search);
      expect(search.total()).toBe(15);
    });
  });

  it("stops looking once it has found as many as it was told to", () => {
    withSearch(
      20,
      5,
      (term, search) => {
        term.write(lines(20_000));
        search.setNeedle("needle");
        const status = search.pump(LONG_MS, 100);
        expect(status).not.toBe("complete");
        expect(search.total()).toBeGreaterThanOrEqual(100);
        expect(search.total()).toBeLessThan(20_000);
        // Asked again, it stays where it is.
        const total = search.total();
        search.pump(LONG_MS, 100);
        expect(search.total()).toBe(total);
      },
      50_000,
    );
  });

  it("stops at the end of the time it was given", () => {
    withSearch(
      20,
      5,
      (term, search) => {
        term.write(lines(20_000));
        search.setNeedle("needle");
        let time = 0;
        const status = search.pump(3, Number.MAX_SAFE_INTEGER, () => time++);
        expect(status).not.toBe("complete");
        expect(time).toBeLessThan(10);
        finish(search);
        expect(search.total()).toBe(20_000);
      },
      50_000,
    );
  });
});

describe("stepping through what a search found", () => {
  it("goes from the newest match to older ones and round to the newest again", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(4));
      search.setNeedle("needle");
      finish(search);
      expect(search.selectedIndex()).toBeUndefined();
      expect(search.next()).toBe(true);
      expect(search.selectedIndex()).toBe(0);
      expect(search.next()).toBe(true);
      expect(search.selectedIndex()).toBe(1);
      search.next();
      search.next();
      expect(search.selectedIndex()).toBe(3);
      search.next();
      expect(search.selectedIndex()).toBe(0);
      expect(search.previous()).toBe(true);
      expect(search.selectedIndex()).toBe(3);
      search.previous();
      expect(search.selectedIndex()).toBe(2);
    });
  });

  it("says which row the selected match is on, counted from the oldest line", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(50));
      search.setNeedle("needle");
      finish(search);
      // The newest is on the last written line: line 49, and a cursor line follows it.
      search.next();
      expect(search.selectedRow()).toBe(49);
      search.next();
      expect(search.selectedRow()).toBe(48);
      search.previous();
      search.previous();
      expect(search.selectedRow()).toBe(0);
    });
  });

  it("goes by where the matches are when the screen lies across two of the library's pages", () => {
    // At this width a page holds 573 rows. With a few rows more than one page, or than
    // two, the rows on screen are the end of one page and the start of the next, and
    // the library then lists the matches of the older page first.
    for (const count of [580, 1153]) {
      withSearch(
        80,
        24,
        (term, search) => {
          term.write(lines(count));
          search.setNeedle("needle");
          finish(search);
          expect(search.total()).toBe(count);
          const rows: number[] = [];
          for (let step = 0; step < count; step++) {
            expect(search.next()).toBe(true);
            expect(search.selectedIndex()).toBe(step);
            rows.push(search.selectedRow()!);
          }
          // From the last line written to the first, with none left out or out of turn.
          expect(rows).toEqual(Array.from({ length: count }, (_, step) => count - 1 - step));
          search.next();
          expect(search.selectedRow()).toBe(count - 1);
          search.previous();
          expect(search.selectedRow()).toBe(0);
          expect(search.selectedIndex()).toBe(count - 1);
        },
        10_000,
      );
    }
  });

  it("marks the match it is at in the viewport, whichever page it is on", () => {
    withSearch(
      80,
      24,
      (term, search) => {
        term.write(lines(580));
        search.setNeedle("needle");
        finish(search);
        const activeRow = () =>
          shown(search)
            .filter((run) => run[3] === HighlightKind.Active)
            .map((run) => run[0]);
        search.next();
        // The last line written is on the row above the cursor's.
        expect(activeRow()).toEqual([22]);
        for (let step = 0; step < 8; step++) search.next();
        expect(activeRow()).toEqual([14]);
      },
      10_000,
    );
  });

  it("does not move the viewport", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(50));
      search.setNeedle("needle");
      finish(search);
      const before = term.scrollbar().offset;
      search.previous();
      expect(search.selectedRow()).toBe(0);
      expect(term.scrollbar().offset).toBe(before);
    });
  });

  it("counts a match found later below the selected one", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(5));
      search.setNeedle("needle");
      finish(search);
      search.next();
      search.next();
      expect(search.selectedIndex()).toBe(1);
      term.write(lines(1));
      finish(search);
      expect(search.total()).toBe(6);
      expect(search.selectedIndex()).toBe(2);
    });
  });
});

describe("the matches in the viewport", () => {
  it("are runs of cells in its rows and columns", () => {
    withSearch(20, 5, (term, search) => {
      term.write("abc needle xyz\r\nnothing here\r\nneedle and needle");
      search.setNeedle("needle");
      expect(shown(search)).toEqual([
        [0, 4, 9, HighlightKind.Match],
        [2, 0, 5, HighlightKind.Match],
        [2, 11, 16, HighlightKind.Match],
      ]);
    });
  });

  it("mark the selected one", () => {
    withSearch(20, 5, (term, search) => {
      term.write("abc needle xyz\r\nneedle");
      search.setNeedle("needle");
      finish(search);
      search.next();
      expect(shown(search)).toEqual([
        [0, 4, 9, HighlightKind.Match],
        [1, 0, 5, HighlightKind.Active],
      ]);
      search.next();
      expect(shown(search)).toEqual([
        [0, 4, 9, HighlightKind.Active],
        [1, 0, 5, HighlightKind.Match],
      ]);
    });
  });

  it("are two runs where a match wraps onto the next row", () => {
    withSearch(10, 5, (term, search) => {
      term.write("1234567needle");
      search.setNeedle("needle");
      expect(shown(search)).toEqual([
        [0, 7, 9, HighlightKind.Match],
        [1, 0, 2, HighlightKind.Match],
      ]);
    });
  });

  it("are cut at the top and the bottom edge", () => {
    withSearch(10, 4, (term, search) => {
      // Rows 0 and 1 hold a match that wraps; twenty lines follow.
      term.write("1234567needle\r\n");
      for (let line = 0; line < 20; line++) term.write(`${line}\r\n`);
      search.setNeedle("needle");
      finish(search);

      // The second row of the match is the first of the viewport.
      term.scrollToRow(1);
      expect(shown(search)).toEqual([[0, 0, 2, HighlightKind.Match]]);

      // Its first row is the last of the viewport.
      term.scrollToTop();
      term.write("");
      expect(shown(search)).toEqual([
        [0, 7, 9, HighlightKind.Match],
        [1, 0, 2, HighlightKind.Match],
      ]);
    });
    withSearch(10, 4, (term, search) => {
      term.write("a\r\nb\r\nc\r\n1234567needle\r\n");
      for (let line = 0; line < 20; line++) term.write(`${line}\r\n`);
      search.setNeedle("needle");
      finish(search);
      term.scrollToTop();
      expect(shown(search)).toEqual([[3, 7, 9, HighlightKind.Match]]);
    });
  });

  it("follow the viewport when it is scrolled", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(50, "x"));
      term.write("needle\r\n");
      term.write(lines(50, "x"));
      search.setNeedle("needle");
      finish(search);
      expect(shown(search)).toEqual([]);
      term.scrollToRow(48);
      expect(shown(search)).toEqual([[2, 0, 5, HighlightKind.Match]]);
    });
  });

  it("are all listed however many there are", () => {
    withSearch(40, 20, (term, search) => {
      for (let line = 0; line < 60; line++) term.write("aa aa aa aa aa aa aa aa aa aa\r\n");
      search.setNeedle("aa");
      const found = shown(search);
      // Nineteen full rows of ten, and the row the cursor is on is empty.
      expect(found).toHaveLength(190);
      for (const [row, first, last] of found) {
        expect(row).toBeGreaterThanOrEqual(0);
        expect(row).toBeLessThan(20);
        expect(last! - first!).toBe(1);
      }
    });
  });
});

describe("a search and what its terminal does", () => {
  it("starts again when the terminal is resized, and is at the same match once it is found", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(30));
      search.setNeedle("needle");
      finish(search);
      for (let step = 0; step < 4; step++) search.next();
      expect(search.selectedIndex()).toBe(3);
      term.resize(12, 5);
      finish(search);
      expect(search.total()).toBe(30);
      expect(search.selectedIndex()).toBe(3);
      search.next();
      expect(search.selectedIndex()).toBe(4);
    });
  });

  it("is at no match once the one it was at has scrolled out of what is kept", () => {
    withSearch(
      20,
      5,
      (term, search) => {
        term.write(lines(20));
        search.setNeedle("needle");
        finish(search);
        search.previous();
        expect(search.selectedRow()).toBe(0);
        term.write(lines(5000, "x"));
        finish(search);
        expect(search.total()).toBe(0);
        expect(search.selectedIndex()).toBeUndefined();
        expect(search.next()).toBe(false);
        term.write("needle");
        expect(search.next()).toBe(true);
        expect(search.selectedIndex()).toBe(0);
      },
      100,
    );
  });

  it("is at no match after another text is looked for, and stays where it was for the same", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(10));
      search.setNeedle("needle");
      finish(search);
      search.next();
      search.next();
      search.setNeedle("NEEDLE");
      expect(search.selectedIndex()).toBe(1);
      search.setNeedle("7 needle");
      finish(search);
      expect(search.selectedIndex()).toBeUndefined();
      search.next();
      expect(search.selectedIndex()).toBe(0);
      expect(search.selectedRow()).toBe(7);
    });
  });

  it("keeps the matches of each screen apart", () => {
    withSearch(20, 5, (term, search) => {
      term.write(lines(30));
      search.setNeedle("needle");
      finish(search);
      term.write(`${ESC}[?1049h${ESC}[Hone needle`);
      finish(search);
      expect(search.total()).toBe(1);
      term.write(`${ESC}[?1049l`);
      finish(search);
      expect(search.total()).toBe(30);
    });
  });

  it("may be freed before its terminal or after it", () => {
    const first = vt.createTerminal(20, 5);
    const early = new Search(first);
    early.setNeedle("x");
    early.free();
    early.free();
    first.write("x");
    first.free();

    const second = vt.createTerminal(20, 5);
    const late = new Search(second);
    second.write("needle");
    late.setNeedle("needle");
    second.free();
    // With its terminal gone a search finds nothing and fails at nothing.
    expect(late.status()).toBe("complete");
    expect(late.pump(1, 10)).toBe("complete");
    expect(late.total()).toBe(0);
    expect(late.next()).toBe(false);
    late.setNeedle("other");
    late.feed();
    const highlights = newHighlights();
    late.readViewport(highlights);
    expect(highlights.count).toBe(0);
    late.free();
  });
});
