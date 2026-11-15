import { beforeAll, describe, expect, it } from "vitest";

import { Mode, type Rgb, type Terminal, type Vt } from "../src/index.ts";
import { loadVt, recordOutput } from "./support.ts";

let vt: Vt;
beforeAll(async () => {
  vt = await loadVt();
});

/** Run `body` with a terminal that is freed afterwards. */
function withTerminal(
  options: Parameters<Vt["createTerminal"]>[2],
  body: (term: Terminal) => void,
): void {
  const term = vt.createTerminal(80, 24, options);
  try {
    body(term);
  } finally {
    term.free();
  }
}

const ESC = "\x1b";
const red: Rgb = { r: 0xcc, g: 0x11, b: 0x22 };
const dark: Rgb = { r: 0x10, g: 0x20, b: 0x30 };

describe("queries in program output", () => {
  it("answers device attributes and the cursor position as replies", () => {
    withTerminal({}, (term) => {
      const sent = recordOutput(term);
      term.write(`ab${ESC}[c${ESC}[6n`);
      expect(sent.replies).toEqual([`${ESC}[?62;22c`, `${ESC}[1;3R`]);
      expect(sent.input).toEqual([]);
    });
  });

  it("answers size queries from the grid and the cell size it was given", () => {
    withTerminal({ cellWidthPx: 9, cellHeightPx: 18 }, (term) => {
      const sent = recordOutput(term);
      term.write(`${ESC}[18t${ESC}[14t${ESC}[16t`);
      expect(sent.replies).toEqual([`${ESC}[8;24;80t`, `${ESC}[4;432;720t`, `${ESC}[6;18;9t`]);
    });
  });

  it("reports the new size after a resize", () => {
    withTerminal({}, (term) => {
      const sent = recordOutput(term);
      term.resize(100, 30, 10, 20);
      term.write(`${ESC}[18t${ESC}[14t`);
      expect(sent.replies).toEqual([`${ESC}[8;30;100t`, `${ESC}[4;600;1000t`]);
    });
  });

  it("answers the colour-scheme query only once a scheme is known", () => {
    withTerminal({}, (term) => {
      const sent = recordOutput(term);
      term.write(`${ESC}[?996n`);
      expect(sent.replies).toEqual([]);
      term.setColorScheme("dark");
      term.write(`${ESC}[?996n`);
      term.setColorScheme("light");
      term.write(`${ESC}[?996n`);
      expect(sent.replies).toEqual([`${ESC}[?997;1n`, `${ESC}[?997;2n`]);
    });
  });

  it("answers colour queries from the colours it was created with", () => {
    withTerminal({ colors: { foreground: red, background: dark } }, (term) => {
      const sent = recordOutput(term);
      term.write(`${ESC}]10;?${ESC}\\${ESC}]11;?${ESC}\\`);
      expect(sent.replies).toEqual([
        `${ESC}]10;rgb:cccc/1111/2222${ESC}\\`,
        `${ESC}]11;rgb:1010/2020/3030${ESC}\\`,
      ]);
    });
  });

  it("answers a palette query from the palette it was given", () => {
    const palette = Array.from({ length: 256 }, (_, i): Rgb => ({ r: i, g: 255 - i, b: 7 }));
    withTerminal({ colors: { palette } }, (term) => {
      const sent = recordOutput(term);
      term.write(`${ESC}]4;3;?${ESC}\\`);
      expect(sent.replies).toEqual([`${ESC}]4;3;rgb:0303/fcfc/0707${ESC}\\`]);
    });
  });

  it("rejects a palette of the wrong length", () => {
    expect(() => vt.createTerminal(80, 24, { colors: { palette: [red] } })).toThrow(RangeError);
  });

  it("does not report the title back, which would inject text as input", () => {
    withTerminal({}, (term) => {
      const sent = recordOutput(term);
      term.write(`${ESC}]2;rm -rf ~${ESC}\\${ESC}[21t`);
      expect(sent.all).toEqual([]);
    });
  });

  it("leaves a clipboard read unanswered", () => {
    withTerminal({ clipboardWriteMaxBytes: 1024 }, (term) => {
      const sent = recordOutput(term);
      term.write(`${ESC}]52;c;?${ESC}\\`);
      expect(sent.all).toEqual([]);
    });
  });
});

describe("effects", () => {
  it("reports the bell, the title and the working directory in order", () => {
    withTerminal({}, (term) => {
      const seen: string[] = [];
      term.handlers = {
        bell: () => seen.push("bell"),
        title: (title) => seen.push(`title ${title}`),
        pwd: (pwd) => seen.push(`pwd ${pwd}`),
      };
      term.write(`${ESC}]2;one${ESC}\\\x07${ESC}]7;file://host/tmp${ESC}\\${ESC}]0;two\x07`);
      expect(seen).toEqual(["title one", "bell", "pwd file://host/tmp", "title two"]);
      expect(term.title()).toBe("two");
      expect(term.pwd()).toBe("file://host/tmp");
    });
  });

  it("delivers effects after the write, so a handler may use the terminal", () => {
    withTerminal({}, (term) => {
      const rows: string[] = [];
      term.handlers = {
        output: () => {
          rows.push(term.rowText(0));
          // Writing from inside a handler would corrupt a terminal still mid-write.
          term.write("!");
        },
      };
      term.write(`before${ESC}[cafter`);
      expect(rows).toEqual(["beforeafter"]);
      expect(term.rowText(0)).toBe("beforeafter!");
    });
  });

  it("delivers effects of a nested write after the ones already queued", () => {
    withTerminal({}, (term) => {
      const order: string[] = [];
      term.handlers = {
        bell: () => {
          order.push("bell");
          if (order.length === 1) term.write(`${ESC}]2;nested${ESC}\\`);
        },
        title: (title) => order.push(title),
      };
      term.write(`\x07${ESC}]2;queued${ESC}\\`);
      expect(order).toEqual(["bell", "queued", "nested"]);
    });
  });

  it("rethrows a handler's error to the caller of write", () => {
    withTerminal({}, (term) => {
      term.handlers = {
        bell: () => {
          throw new Error("handler failed");
        },
      };
      expect(() => term.write("\x07")).toThrow("handler failed");
      // The terminal is still usable.
      term.handlers = {};
      term.write("ok");
      expect(term.rowText(0)).toBe("ok");
    });
  });

  it("passes on a clipboard write within the cap", () => {
    withTerminal({ clipboardWriteMaxBytes: 16 }, (term) => {
      const writes: string[] = [];
      term.handlers = { clipboardWrite: (text, location) => writes.push(`${location}:${text}`) };
      term.write(`${ESC}]52;c;${btoa("héllo")}${ESC}\\`);
      term.write(`${ESC}]52;p;${btoa("x")}${ESC}\\`);
      expect(writes).toEqual([`standard:${latin1("héllo")}`, "primary:x"]);
    });
  });

  it("refuses a clipboard write above the cap", () => {
    withTerminal({ clipboardWriteMaxBytes: 16 }, (term) => {
      const writes: string[] = [];
      term.handlers = { clipboardWrite: (text) => writes.push(text) };
      term.write(`${ESC}]52;c;${btoa("x".repeat(17))}${ESC}\\`);
      expect(writes).toEqual([]);
      term.write(`${ESC}]52;c;${btoa("x".repeat(16))}${ESC}\\`);
      expect(writes).toEqual(["x".repeat(16)]);
    });
  });

  it("refuses every clipboard write when no cap was given", () => {
    withTerminal({}, (term) => {
      const writes: string[] = [];
      term.handlers = { clipboardWrite: (text) => writes.push(text) };
      term.write(`${ESC}]52;c;${btoa("x")}${ESC}\\`);
      expect(writes).toEqual([]);
    });
  });
});

describe("terminal state", () => {
  it("reports modes a program set", () => {
    withTerminal({}, (term) => {
      expect(term.mode(Mode.BracketedPaste)).toBe(false);
      expect(term.mouseTracking()).toBe(false);
      term.write(`${ESC}[?2004h${ESC}[?1000h${ESC}[4h`);
      expect(term.mode(Mode.BracketedPaste)).toBe(true);
      expect(term.mode(Mode.MouseNormal)).toBe(true);
      expect(term.mouseTracking()).toBe(true);
      expect(term.mode(4, true)).toBe(true);
      expect(term.mode(4)).toBe(false);
    });
  });

  it("changes its revision when modes may have changed", () => {
    withTerminal({}, (term) => {
      const before = term.revision;
      term.write("x");
      const afterWrite = term.revision;
      term.reset();
      expect(afterWrite).not.toBe(before);
      expect(term.revision).not.toBe(afterWrite);
    });
  });

  it("scrolls the viewport through scrollback and back", () => {
    withTerminal({ scrollbackMaxLines: 1000, scrollbackMaxBytes: 1 << 20 }, (term) => {
      term.write(Array.from({ length: 100 }, (_, i) => `line ${i}`).join("\r\n"));
      const bottom = term.scrollbar();
      expect(bottom.length).toBe(24);
      expect(bottom.total).toBe(100);
      expect(bottom.offset).toBe(76);
      expect(term.viewportAtBottom()).toBe(true);

      term.scrollBy(-10);
      expect(term.scrollbar().offset).toBe(66);
      expect(term.viewportAtBottom()).toBe(false);
      term.scrollToTop();
      expect(term.scrollbar().offset).toBe(0);
      term.scrollToRow(40);
      expect(term.scrollbar().offset).toBe(40);
      term.scrollToBottom();
      expect(term.viewportAtBottom()).toBe(true);
    });
  });

  it("keeps scrollback within the limits it was given", () => {
    // The limits are applied a page at a time, so the kept history is close to the
    // limit, not equal to it.
    withTerminal({ scrollbackMaxLines: 200, scrollbackMaxBytes: 64 << 20 }, (term) => {
      term.write("x\r\n".repeat(5000));
      const kept = term.scrollbar().total;
      expect(kept).toBeGreaterThanOrEqual(200);
      expect(kept).toBeLessThan(2500);
    });
  });

  it("keeps more history with a byte limit above the default", () => {
    const history = (options: Parameters<Vt["createTerminal"]>[2]) => {
      let total = 0;
      withTerminal(options, (term) => {
        term.write("x\r\n".repeat(20000));
        total = term.scrollbar().total;
      });
      return total;
    };
    // A line limit alone is still held back by the byte limit's small default. With
    // both set the history reaches the line limit, to within the page it is trimmed by.
    const linesOnly = history({ scrollbackMaxLines: 10000 });
    const both = history({ scrollbackMaxLines: 10000, scrollbackMaxBytes: 64 << 20 });
    expect(both).toBeGreaterThan(linesOnly * 2);
    expect(both).toBeGreaterThan(9000);
    expect(both).toBeLessThan(12000);
  });

  it("forgets its scrollback when asked, keeps the screen, and goes on keeping history", () => {
    const lines = (from: number, count: number) =>
      Array.from({ length: count }, (_, i) => `line ${from + i}`).join("\r\n");
    for (const options of [{ scrollbackMaxLines: 1000, scrollbackMaxBytes: 4 << 20 }, {}]) {
      withTerminal(options, (term) => {
        term.write(lines(0, 100));
        expect(term.scrollbar().total).toBe(100);
        const screen = term.screenText();
        const revision = term.revision;

        term.clearScrollback();
        expect(term.scrollbar()).toEqual({ total: 24, offset: 0, length: 24 });
        expect(term.viewportAtBottom()).toBe(true);
        expect(term.screenText()).toEqual(screen);
        expect(term.revision).not.toBe(revision);

        // The limit was put back, so history builds up again.
        term.write("\r\n" + lines(100, 50));
        expect(term.scrollbar().total).toBe(74);
        expect(term.rowText(23)).toBe("line 149");
      });
    }
  });

  it("forgets its scrollback while the viewport is showing it", () => {
    withTerminal({ scrollbackMaxLines: 1000, scrollbackMaxBytes: 4 << 20 }, (term) => {
      term.write(Array.from({ length: 100 }, (_, i) => `line ${i}`).join("\r\n"));
      term.scrollToTop();
      term.clearScrollback();
      expect(term.scrollbar()).toEqual({ total: 24, offset: 0, length: 24 });
      expect(term.viewportAtBottom()).toBe(true);
    });
  });

  it("refuses use after free", () => {
    const term = vt.createTerminal(10, 2);
    term.free();
    term.free();
    expect(() => term.write("x")).toThrow(/freed/);
    expect(() => term.cursor()).toThrow(/freed/);
  });
});

describe("many terminals", () => {
  it("leaves the function table and memory steady across create and free", () => {
    const { x } = vt.core;
    // Warm up, so one-off growth is not counted.
    for (let i = 0; i < 10; i++) vt.createTerminal(80, 24, {}).free();
    const table = x.__indirect_function_table.length;
    const memory = x.memory.buffer.byteLength;
    for (let i = 0; i < 200; i++) {
      const term = vt.createTerminal(80, 24, { scrollbackMaxLines: 100 });
      term.handlers = { output: () => undefined };
      term.write(`hello ${i}${ESC}[c\r\n`);
      term.free();
    }
    expect(x.__indirect_function_table.length).toBe(table);
    expect(x.memory.buffer.byteLength).toBe(memory);
  });
});

/** The string whose UTF-16 units are the bytes `btoa` encoded. */
function latin1(text: string): string {
  return new TextDecoder().decode(Uint8Array.from(text, (ch) => ch.charCodeAt(0)));
}
