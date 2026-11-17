import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { BRAND_SLUGS, generate, LINE_NAMES } from "../scripts/icons.ts";
import { BRAND_GLYPHS, LINE_GLYPHS } from "../src/icons/glyphs.generated.ts";
import { BRAND_TILES } from "../src/icons/tiles.ts";

describe("the copied glyphs", () => {
  it("are what the script writes from the installed packages", () => {
    for (const [path, text] of Object.entries(generate())) {
      const kept = readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
      // Run `pnpm --filter @lume/ui icons` when this fails.
      expect(kept.replace(/\r\n/g, "\n"), path).toBe(text);
    }
  });

  it("hold every name the script asks for, and every brand a tile draws", () => {
    expect(Object.keys(LINE_GLYPHS).sort()).toEqual([...LINE_NAMES].sort());
    expect(Object.keys(BRAND_GLYPHS).sort()).toEqual([...BRAND_SLUGS].sort());
    for (const slug of Object.values(BRAND_TILES)) expect(BRAND_GLYPHS).toHaveProperty(slug);
  });
});
