import { beforeAll, describe, expect, it } from "vitest";

import {
  clearSelection,
  DirtyState,
  formatText,
  hasSelection,
  hyperlinkAt,
  NO_SELECTION,
  RenderState,
  selectAll,
  SelectionGesture,
  selectionText,
  type PointerAt,
  type Terminal,
  type Vt,
} from "../src/index.ts";
import { loadVt } from "./support.ts";

let vt: Vt;
beforeAll(async () => {
  vt = await loadVt();
});

const ESC = "\x1b";
const CELL_W = 10;
const CELL_H = 20;

/**
 * The pointer over a cell of the viewport. `across` is how far across the cell it is: a
 * drag takes in the cell it ends on only once the pointer is well past its middle.
 */
function cell(col: number, row: number, across = 0.2): PointerAt {
  return { col, row, x: (col + across) * CELL_W, y: row * CELL_H + CELL_H / 2 };
}

function withGesture(
  cols: number,
  rows: number,
  body: (term: Terminal, gesture: SelectionGesture) => void,
): void {
  const term = vt.createTerminal(cols, rows, { scrollbackMaxLines: 1000 });
  const gesture = new SelectionGesture(term);
  gesture.setGeometry({
    columns: cols,
    cellWidth: CELL_W,
    paddingLeft: 0,
    screenHeight: rows * CELL_H,
  });
  gesture.setClickTiming({ repeatIntervalMs: 500, repeatDistancePx: CELL_W });
  try {
    body(term, gesture);
  } finally {
    gesture.free();
    term.free();
  }
}

describe("selecting with the pointer", () => {
  it("selects the cells a drag covers", () => {
    withGesture(20, 5, (term, gesture) => {
      term.write("hello world");
      gesture.press(cell(0, 0), { timeMs: 0 });
      expect(hasSelection(term)).toBe(false);
      gesture.drag(cell(4, 0, 0.2));
      expect(selectionText(term)).toBe("hell");
      gesture.drag(cell(4, 0, 0.8));
      gesture.release(cell(4, 0, 0.8));
      expect(hasSelection(term)).toBe(true);
      expect(selectionText(term)).toBe("hello");
    });
  });

  it("selects a word on a double click and the line on a triple click", () => {
    withGesture(20, 5, (term, gesture) => {
      term.write("hello world");
      gesture.press(cell(8, 0), { timeMs: 1000 });
      gesture.release(cell(8, 0));
      gesture.press(cell(8, 0), { timeMs: 1100 });
      expect(gesture.clickCount()).toBe(2);
      expect(selectionText(term)).toBe("world");
      gesture.release(cell(8, 0));
      gesture.press(cell(8, 0), { timeMs: 1200 });
      expect(gesture.clickCount()).toBe(3);
      expect(selectionText(term)).toBe("hello world");
    });
  });

  it("counts a slow or distant second press as a new click", () => {
    withGesture(20, 5, (term, gesture) => {
      term.write("hello world");
      gesture.press(cell(8, 0), { timeMs: 1000 });
      gesture.release(cell(8, 0));
      gesture.press(cell(8, 0), { timeMs: 1600 });
      expect(gesture.clickCount()).toBe(1);
      gesture.release(cell(8, 0));
      gesture.press(cell(2, 0), { timeMs: 1650 });
      expect(gesture.clickCount()).toBe(1);
      expect(hasSelection(term)).toBe(false);
    });
  });

  it("treats presses without a time as single clicks", () => {
    withGesture(20, 5, (term, gesture) => {
      term.write("hello world");
      gesture.press(cell(8, 0));
      gesture.release(cell(8, 0));
      gesture.press(cell(8, 0));
      expect(hasSelection(term)).toBe(false);
    });
  });

  it("clears the selection on a plain click", () => {
    withGesture(20, 5, (term, gesture) => {
      term.write("hello world");
      selectAll(term);
      expect(hasSelection(term)).toBe(true);
      gesture.press(cell(2, 0), { timeMs: 5000 });
      gesture.release(cell(2, 0));
      expect(hasSelection(term)).toBe(false);
    });
  });

  it("selects a rectangle when asked", () => {
    withGesture(20, 5, (term, gesture) => {
      term.write("abcdef\r\nghijkl\r\nmnopqr");
      gesture.press(cell(1, 0), { timeMs: 0 });
      gesture.drag(cell(3, 2, 0.8), { rectangle: true });
      gesture.release(cell(3, 2, 0.8));
      expect(selectionText(term)).toBe("bcd\nhij\nnop");
    });
  });

  it("asks for scrolling when the drag leaves the grid, and scrolls a row per tick", () => {
    withGesture(20, 5, (term, gesture) => {
      term.write(Array.from({ length: 30 }, (_, i) => `line ${i}`).join("\r\n"));
      const top = term.scrollbar().offset;
      gesture.press(cell(3, 2), { timeMs: 0 });
      expect(gesture.drag(cell(3, 1))).toBe("none");
      const above: PointerAt = { col: 3, row: 0, x: 35, y: -15 };
      expect(gesture.drag(above)).toBe("up");
      expect(gesture.autoscrollTick(above)).toBe("up");
      expect(term.scrollbar().offset).toBe(top - 1);
      expect(gesture.autoscrollTick(above)).toBe("up");
      expect(term.scrollbar().offset).toBe(top - 2);
      gesture.release();
      expect(selectionText(term).split("\n").length).toBeGreaterThanOrEqual(4);
    });
  });
});

describe("selected text", () => {
  it("joins a wrapped line back together", () => {
    withGesture(10, 4, (term) => {
      term.write("abcdefghijklmno");
      selectAll(term);
      expect(selectionText(term)).toBe("abcdefghijklmno");
    });
  });

  it("keeps wide characters whole and drops trailing blanks", () => {
    withGesture(10, 4, (term) => {
      term.write("a中b   \r\n日本");
      selectAll(term);
      expect(selectionText(term)).toBe("a中b\n日本");
    });
  });

  it("is empty when nothing is selected", () => {
    withGesture(10, 4, (term) => {
      term.write("abc");
      expect(selectionText(term)).toBe("");
      selectAll(term);
      clearSelection(term);
      expect(selectionText(term)).toBe("");
    });
  });

  it("includes scrollback in select all", () => {
    withGesture(10, 3, (term) => {
      term.write("one\r\ntwo\r\nthree\r\nfour\r\nfive");
      selectAll(term);
      expect(selectionText(term)).toBe("one\ntwo\nthree\nfour\nfive");
    });
  });
});

describe("the selection in a frame", () => {
  it("gives each row its selected range and marks the rows that changed", () => {
    withGesture(20, 4, (term, gesture) => {
      term.write("hello world\r\nsecond line");
      const state = new RenderState(term);
      try {
        const frame = state.update();
        expect([...frame.selectionStart]).toEqual([-1, -1, -1, -1]);
        state.markDrawn();

        gesture.press(cell(6, 0), { timeMs: 0 });
        gesture.drag(cell(2, 1, 0.8));
        state.update();
        expect(frame.dirty).not.toBe(DirtyState.None);
        expect([frame.selectionStart[0], frame.selectionEnd[0]]).toEqual([6, 19]);
        expect([frame.selectionStart[1], frame.selectionEnd[1]]).toEqual([0, 2]);
        expect(frame.selectionStart[2]).toBe(NO_SELECTION);
        state.markDrawn();

        clearSelection(term);
        state.update();
        expect(frame.dirty).not.toBe(DirtyState.None);
        expect([...frame.selectionStart]).toEqual([-1, -1, -1, -1]);
      } finally {
        state.free();
      }
    });
  });
});

describe("hyperlinks", () => {
  it("reports the target of the link under a cell", () => {
    withGesture(20, 2, (term) => {
      term.write(`a${ESC}]8;;https://example.com/x?y=1${ESC}\\bc${ESC}]8;;${ESC}\\d`);
      expect(hyperlinkAt(term, 0, 0)).toBe("");
      expect(hyperlinkAt(term, 1, 0)).toBe("https://example.com/x?y=1");
      expect(hyperlinkAt(term, 2, 0)).toBe("https://example.com/x?y=1");
      expect(hyperlinkAt(term, 3, 0)).toBe("");
    });
  });

  it("reports a target longer than its first buffer", () => {
    withGesture(20, 2, (term) => {
      const target = `https://example.com/${"p".repeat(2000)}`;
      term.write(`${ESC}]8;;${target}${ESC}\\x${ESC}]8;;${ESC}\\`);
      expect(hyperlinkAt(term, 0, 0)).toBe(target);
    });
  });
});

describe("formatting the screen", () => {
  it("gives the screen as plain text", () => {
    withGesture(10, 3, (term) => {
      term.write(`${ESC}[31mred${ESC}[0m\r\nplain`);
      expect(formatText(term, { trim: true })).toBe("red\nplain");
    });
  });

  it("can keep the styling as escape sequences or as markup", () => {
    withGesture(10, 3, (term) => {
      term.write(`${ESC}[31mred${ESC}[0m`);
      expect(formatText(term, { format: "vt" })).toContain(`${ESC}[`);
      expect(formatText(term, { format: "html" })).toMatch(/<\w+/);
    });
  });
});
