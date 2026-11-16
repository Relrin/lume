import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { DEFAULT_TOKENS, LIGHT_TOKENS, terminalAppearance } from "@lume/tokens";
import { describe, expect, it } from "vitest";

import { SCROLLBACK_DEFAULT_LINES, SCROLLBACK_MAX_LINES } from "../src/limits.ts";
import { contrast, DEFAULT_APPEARANCE, rendererTheme } from "../src/theme.ts";

const BLACK = { r: 0, g: 0, b: 0 };
const WHITE = { r: 255, g: 255, b: 255 };

/** The channels of a colour packed as a frame packs it. */
function unpack(color: number) {
  return { r: color & 0xff, g: (color >>> 8) & 0xff, b: (color >>> 16) & 0xff, a: color >>> 24 };
}

describe("how well two colours can be told apart", () => {
  it("is least for a colour and itself and most for black and white", () => {
    expect(contrast(WHITE, WHITE)).toBe(1);
    expect(contrast(BLACK, WHITE)).toBeCloseTo(21, 5);
    expect(contrast(WHITE, BLACK)).toBeCloseTo(21, 5);
  });
});

describe("what a search is drawn with", () => {
  it("keeps the opacity of the colour behind what was found", () => {
    const theme = rendererTheme(DEFAULT_APPEARANCE, 14);
    expect(unpack(theme.searchMatch)).toEqual(DEFAULT_APPEARANCE.searchMatch);
    expect(unpack(theme.searchMatch).a).toBeLessThan(255);
    expect(unpack(theme.searchActive).a).toBe(255);
  });

  it("writes the match a search is at in whichever of the terminal's colours shows better", () => {
    const looks = [
      terminalAppearance(DEFAULT_TOKENS, "dark"),
      terminalAppearance(LIGHT_TOKENS, "light"),
    ];
    for (const look of looks) {
      const theme = rendererTheme(look, 14);
      const text = unpack(theme.searchActiveText!);
      const behind = look.searchActive;
      const other = contrast(look.foreground, behind) >= contrast(look.background, behind);
      expect(contrast(text, behind)).toBeGreaterThanOrEqual(
        contrast(other ? look.background : look.foreground, behind),
      );
      expect(contrast(text, behind)).toBeGreaterThan(4.5);
    }
  });
});

describe("how many lines a terminal keeps", () => {
  it("is at most what the configuration may ask for, and by default what it asks for", () => {
    const path = fileURLToPath(new URL("../../../schemas/config.schema.json", import.meta.url));
    const schema = JSON.parse(readFileSync(path, "utf8")) as {
      properties: {
        terminal: { properties: Record<string, { maximum: number; default: number }> };
      };
    };
    const lines = schema.properties.terminal.properties["scrollback-lines"]!;
    expect(SCROLLBACK_MAX_LINES).toBe(lines.maximum);
    expect(SCROLLBACK_DEFAULT_LINES).toBe(lines.default);
  });
});
