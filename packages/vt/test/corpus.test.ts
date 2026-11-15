import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { dump, parseScript, replay } from "../src/index.ts";
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

    const term = replay(await loadVt(), script, input);
    try {
      expect(dump(term)).toBe(expected);
    } finally {
      term.free();
    }
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
