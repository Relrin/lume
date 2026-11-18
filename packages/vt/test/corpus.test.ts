import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { dump, formatText, parseScript, replay, type Terminal } from "../src/index.ts";
import { corpusDir, loadVt } from "./support.ts";

/** Every recording directory, sorted by name, as the native suite lists them. */
async function recordings(): Promise<string[]> {
  const entries = await readdir(corpusDir, { withFileTypes: true });
  return entries
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();
}

describe("the golden corpus through the WebAssembly build", async () => {
  const names = await recordings();

  it("finds the recordings", () => {
    expect(names.length).toBeGreaterThanOrEqual(5);
  });

  // The expected dumps were written by the native build. Matching them byte for byte is
  // what shows the two builds agree.
  it.each(names)("%s produces the native dump", async (name) => {
    const dir = join(corpusDir, name);
    const script = parseScript(await readFile(join(dir, "script.toml"), "utf8"));
    const input = await readFile(join(dir, "input.bin"));
    const expected = (await readFile(join(dir, "expect.txt"), "utf8")).replaceAll("\r\n", "\n");

    const { terminal, replies } = replay(await loadVt(), script, input);
    try {
      expect(dump(terminal, replies)).toBe(expected);
    } finally {
      terminal.free();
    }
  });

  // A snapshot the native build saved at the end of each recording loads here and shows
  // the same screen. What the terminal answered is not part of a snapshot.
  it.each(names)("%s loads from the native snapshot", async (name) => {
    const dir = join(corpusDir, name);
    const snapshot = await readFile(join(dir, "snapshot.bin"));
    const expected = (await readFile(join(dir, "expect.txt"), "utf8")).replaceAll("\r\n", "\n");
    const withoutReplies = (text: string) => text.slice(0, text.indexOf("--- replies"));

    const terminal = (await loadVt()).loadTerminal(new Uint8Array(snapshot));
    try {
      expect(withoutReplies(dump(terminal))).toBe(withoutReplies(expected));
    } finally {
      terminal.free();
    }
  });

  it("restores only the newest history when asked to", async () => {
    // 3000 numbered lines in a 40×5 terminal, saved by the native build.
    const snapshot = new Uint8Array(
      await readFile(join(corpusDir, "..", "hydrate", "history.bin")),
    );
    const vt = await loadVt();
    const lines = (terminal: Terminal) =>
      formatText(terminal, { format: "plain" })
        .split("\n")
        .filter((line) => line.startsWith("history-"));

    const all = vt.loadTerminal(snapshot);
    const some = vt.loadTerminal(snapshot, { historyLines: 200 });
    try {
      const everything = lines(all);
      const kept = lines(some);
      expect(everything[0]).toBe("history-0001");
      expect(everything.at(-1)).toBe("history-3000");
      // Whole pages are restored, so a little more than asked for, and far from all;
      // the newest lines are the ones kept, and the screen is whole.
      expect(kept.length).toBeGreaterThanOrEqual(200);
      expect(kept.length).toBeLessThan(everything.length / 2);
      expect(kept).toEqual(everything.slice(-kept.length));
      expect(some.screenText()).toEqual(all.screenText());
      expect(some.leftHistoryOut).toBe(true);
      expect(all.leftHistoryOut).toBe(false);
    } finally {
      all.free();
      some.free();
    }
  });

  it("refuses a damaged snapshot", async () => {
    const vt = await loadVt();
    expect(() => vt.loadTerminal(new Uint8Array([1, 2, 3, 4]))).toThrow();
  });
});

describe("recording scripts", () => {
  it("reads the size, the description and the resizes", () => {
    const script = parseScript(
      [
        "# a comment",
        'description = "wraps, then \\"reflows\\""',
        "cols = 40",
        "rows = 12",
        "",
        "[[resize]]",
        "at = 149",
        "cols = 20",
        "rows = 12",
      ].join("\n"),
    );
    expect(script).toEqual({
      description: 'wraps, then "reflows"',
      cols: 40,
      rows: 12,
      resize: [{ at: 149, cols: 20, rows: 12 }],
    });
  });

  it("rejects an unknown key", () => {
    expect(() => parseScript('description = "x"\ncols = 1\nrows = 1\ncolour = 2')).toThrow(
      /unknown key colour/,
    );
  });

  it("rejects a missing key", () => {
    expect(() => parseScript('description = "x"\ncols = 1')).toThrow(/rows must be a number/);
  });

  it("rejects a table it does not know", () => {
    expect(() => parseScript('description = "x"\ncols = 1\nrows = 1\n[other]')).toThrow(
      /unknown table/,
    );
  });

  it("rejects a resize beyond the input", async () => {
    const vt = await loadVt();
    const script = parseScript(
      'description = "x"\ncols = 4\nrows = 2\n[[resize]]\nat = 9\ncols = 2\nrows = 2',
    );
    expect(() => replay(vt, script, new Uint8Array(3))).toThrow(/within the input/);
  });
});
