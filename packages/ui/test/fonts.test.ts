import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

import { describe, expect, it, vi } from "vitest";

import { loadTerminalFonts, terminalFaces } from "../src/config/fonts.ts";
import { DEFAULT_APPEARANCE } from "../src/theme.ts";
import { FONT_LOAD_TIMEOUT_MS } from "../src/tuning.ts";

const FONTS = new URL("../src/fonts/", import.meta.url);

describe("the fonts shipped with the application", () => {
  it("are the files their checksums were taken from", () => {
    const sums = readFileSync(new URL("SHA256SUMS", FONTS), "utf8").trim().split("\n");
    expect(sums).toHaveLength(6);
    for (const line of sums) {
      const [sum, name] = line.split(/\s+/);
      const bytes = readFileSync(new URL(name ?? "", FONTS));
      expect(createHash("sha256").update(bytes).digest("hex"), name).toBe(sum);
      // A web font in the compressed form begins with these four characters.
      expect(bytes.subarray(0, 4).toString("latin1"), name).toBe("wOF2");
    }
  });

  it("each have a face declared for them, and come with their licence", () => {
    const css = readFileSync(new URL("fonts.css", FONTS), "utf8");
    const sums = readFileSync(new URL("SHA256SUMS", FONTS), "utf8");
    const declared = [...css.matchAll(/url\("\.\/([^"]+)"\)/g)].map((match) => match[1]);
    expect(declared.sort()).toEqual(
      [...sums.matchAll(/ {2}(\S+)$/gm)].map((match) => match[1]).sort(),
    );
    for (const licence of ["OFL.txt", "OFL-Inter.txt"]) {
      expect(readFileSync(new URL(licence, FONTS), "utf8")).toContain("SIL Open Font License");
    }
  });
});

describe("a terminal's font before a terminal is drawn", () => {
  it("is asked for in the four faces a terminal draws with", async () => {
    const asked: string[] = [];
    await loadTerminalFonts(DEFAULT_APPEARANCE, {
      load: (font) => {
        asked.push(font);
        return Promise.resolve([]);
      },
    });
    const family = DEFAULT_APPEARANCE.fontFamily;
    expect(asked).toEqual([
      `400 14px ${family}`,
      `700 14px ${family}`,
      `italic 400 14px ${family}`,
      `italic 700 14px ${family}`,
    ]);
    expect(terminalFaces({ ...DEFAULT_APPEARANCE, fontSizePx: 11, fontWeight: 500 })[0]).toBe(
      `500 11px ${family}`,
    );
  });

  it("is not waited for when it cannot be had", async () => {
    await expect(
      loadTerminalFonts(DEFAULT_APPEARANCE, { load: () => Promise.reject(new Error("no")) }),
    ).resolves.toBeUndefined();
  });

  it("is not waited for longer than a start may be held up", async () => {
    vi.useFakeTimers();
    try {
      let done = false;
      const loading = loadTerminalFonts(DEFAULT_APPEARANCE, {
        load: () => new Promise(() => undefined),
      }).then(() => (done = true));
      await vi.advanceTimersByTimeAsync(FONT_LOAD_TIMEOUT_MS - 1);
      expect(done).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      await loading;
      expect(done).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });
});
