import { CellFlag } from "@lume/renderer";
import { makeFrame, putText } from "@lume/renderer/testing";
import { describe, expect, it } from "vitest";

import { addressAt, addressUnder, linkScheme, markedRun } from "../src/terminal/links.ts";

describe("the kinds of link that are opened", () => {
  it("are web addresses and files", () => {
    expect(linkScheme("http://example.org")).toBe("http");
    expect(linkScheme("HTTPS://example.org")).toBe("https");
    expect(linkScheme("file:///home/someone/notes.txt")).toBe("file");
  });

  it("are nothing else", () => {
    for (const target of [
      "javascript:alert(1)",
      "data:text/html,hello",
      "ftp://example.org",
      "mailto:someone@example.org",
      "ssh://example.org",
      "vbscript:x",
      "example.org",
      "//example.org",
      " https://example.org",
      "",
    ]) {
      expect(linkScheme(target), target).toBeNull();
    }
  });
});

describe("an address in running text", () => {
  const at = (text: string, index: number) => addressAt(text, index)?.target ?? null;

  it("is found from any character inside it and from none outside", () => {
    const text = "see https://example.org/a now";
    expect(at(text, 4)).toBe("https://example.org/a");
    expect(at(text, 24)).toBe("https://example.org/a");
    expect(at(text, 3)).toBeNull();
    expect(at(text, 25)).toBeNull();
    expect(at("no address here", 3)).toBeNull();
  });

  it("does not take in the punctuation of the sentence around it", () => {
    expect(at("Read https://example.org/a.", 10)).toBe("https://example.org/a");
    expect(at("Is it https://example.org/a?", 10)).toBe("https://example.org/a");
    expect(at("(see https://example.org/a)", 10)).toBe("https://example.org/a");
    expect(at("[https://example.org/a], and", 10)).toBe("https://example.org/a");
    expect(at("https://example.org/a...", 3)).toBe("https://example.org/a");
  });

  it("keeps punctuation that belongs to the address", () => {
    expect(at("https://example.org/a?b=c&d=e#f", 3)).toBe("https://example.org/a?b=c&d=e#f");
    expect(at("https://en.example.org/wiki/Shell_(computing)", 3)).toBe(
      "https://en.example.org/wiki/Shell_(computing)",
    );
    expect(at("(https://en.example.org/wiki/Shell_(computing))", 3)).toBe(
      "https://en.example.org/wiki/Shell_(computing)",
    );
    expect(at("https://example.org:8080/a.b", 3)).toBe("https://example.org:8080/a.b");
  });

  it("stops at quotes, brackets and spaces", () => {
    expect(at('"https://example.org/a"', 3)).toBe("https://example.org/a");
    expect(at("<https://example.org/a>", 3)).toBe("https://example.org/a");
    expect(at("https://example.org/a https://example.org/b", 30)).toBe("https://example.org/b");
  });

  it("is only of a kind that is opened", () => {
    expect(at("ftp://example.org/a", 3)).toBeNull();
    expect(at("file:///home/someone/a.txt", 3)).toBe("file:///home/someone/a.txt");
    // A scheme has to start a word.
    expect(at("xhttps://example.org", 5)).toBeNull();
  });
});

describe("the address under a cell", () => {
  it("covers the cells that show it", () => {
    const frame = makeFrame(40, 4);
    putText(frame, 2, 1, "at https://example.org/a.");
    expect(addressUnder(frame, 10, 1)).toEqual({
      target: "https://example.org/a",
      ranges: [{ row: 1, startCol: 5, endCol: 25 }],
    });
    expect(addressUnder(frame, 3, 1)).toBeNull();
    expect(addressUnder(frame, 26, 1)).toBeNull();
    expect(addressUnder(frame, 10, 2)).toBeNull();
  });

  it("is nothing outside the grid", () => {
    const frame = makeFrame(40, 4);
    expect(addressUnder(frame, -1, 0)).toBeNull();
    expect(addressUnder(frame, 40, 0)).toBeNull();
    expect(addressUnder(frame, 0, 4)).toBeNull();
  });

  it("follows an address that wraps onto the next row, from either row", () => {
    const frame = makeFrame(20, 4);
    putText(frame, 0, 1, "go https://example.o");
    putText(frame, 0, 2, "rg/path end");
    const expected = {
      target: "https://example.org/path",
      ranges: [
        { row: 1, startCol: 3, endCol: 19 },
        { row: 2, startCol: 0, endCol: 6 },
      ],
    };
    expect(addressUnder(frame, 8, 1)).toEqual(expected);
    expect(addressUnder(frame, 2, 2)).toEqual(expected);
  });

  it("does not join a row that ends short of the edge to the next", () => {
    const frame = makeFrame(30, 4);
    putText(frame, 0, 1, "https://example.org/a");
    putText(frame, 0, 2, "b/c");
    expect(addressUnder(frame, 3, 1)?.target).toBe("https://example.org/a");
    expect(addressUnder(frame, 1, 2)).toBeNull();
  });

  it("counts a wide character as the two cells it takes", () => {
    const frame = makeFrame(40, 2);
    putText(frame, 0, 0, "漢 https://example.org/字 x");
    const hit = addressUnder(frame, 5, 0);
    expect(hit?.target).toBe("https://example.org/字");
    // The wide character before the address moves it right by one cell, and the one
    // that ends it takes its second cell with it.
    expect(hit?.ranges).toEqual([{ row: 0, startCol: 3, endCol: 24 }]);
    // The second cell of that character is part of the address too.
    expect(addressUnder(frame, 24, 0)?.target).toBe("https://example.org/字");
  });
});

describe("the cells of a link a program marked", () => {
  it("are the neighbours marked with the same target", () => {
    const frame = makeFrame(40, 2);
    putText(frame, 0, 0, "one two three");
    const targets = new Map<number, string>();
    const mark = (from: number, to: number, target: string) => {
      for (let col = from; col <= to; col++) {
        frame.flags[col] = frame.flags[col]! | CellFlag.Hyperlink;
        targets.set(col, target);
      }
    };
    mark(0, 2, "https://example.org/one");
    mark(3, 7, "https://example.org/two");
    const targetAt = (col: number) => targets.get(col) ?? "";

    expect(markedRun(frame, 5, 0, "https://example.org/two", targetAt)).toEqual({
      row: 0,
      startCol: 3,
      endCol: 7,
    });
    // A neighbouring link with another target is not part of it.
    expect(markedRun(frame, 1, 0, "https://example.org/one", targetAt)).toEqual({
      row: 0,
      startCol: 0,
      endCol: 2,
    });
  });
});
