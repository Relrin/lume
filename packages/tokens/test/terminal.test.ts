import { describe, expect, it } from "vitest";

import {
  DEFAULT_SCHEME,
  DEFAULT_TOKENS,
  parseColor,
  terminalAppearance,
  type Tokens,
} from "../src/index.ts";

describe("reading a colour", () => {
  it("takes six digits, or eight with the opacity last, in either case", () => {
    expect(parseColor("#1c1f2a")).toEqual({ r: 0x1c, g: 0x1f, b: 0x2a, a: 255 });
    expect(parseColor("#1C1F2A80")).toEqual({ r: 0x1c, g: 0x1f, b: 0x2a, a: 0x80 });
    expect(parseColor("#000000")).toEqual({ r: 0, g: 0, b: 0, a: 255 });
  });

  it("refuses what is not a colour", () => {
    for (const bad of ["1c1f2a", "#fff", "#1c1f2g", "red", "#1c1f2a8", ""]) {
      expect(() => parseColor(bad), bad).toThrow(/is not a colour/);
    }
  });
});

describe("a terminal's look", () => {
  it("is the built-in dark theme by default", () => {
    const look = terminalAppearance(DEFAULT_TOKENS, DEFAULT_SCHEME);
    expect(look.scheme).toBe("dark");
    expect(look.background).toEqual({ r: 0x17, g: 0x1a, b: 0x1e });
    expect(look.foreground).toEqual({ r: 0xd7, g: 0xda, b: 0xdd });
    expect(look.cursorText).toEqual({ r: 0x0f, g: 0x11, b: 0x13 });
    expect(look.selectionBackground).toEqual({ r: 0x2b, g: 0x4a, b: 0x40 });
    expect(look.searchMatch.a).toBe(0x66);
    expect(look.searchActive.a).toBe(255);
    expect(look.ansi).toHaveLength(16);
    expect(look.ansi[1]).toEqual({ r: 0xff, g: 0x8b, b: 0x82 });
    expect(look.ansi[15]).toEqual({ r: 255, g: 255, b: 255 });
    expect(look.fontFamily).toBe(
      '"JetBrains Mono", "Cascadia Mono", "Consolas", "Menlo", "DejaVu Sans Mono", monospace',
    );
    expect(look.fontSizePx).toBe(14);
    expect(look.lineHeight).toBe(1.2);
    expect(look.fontWeight).toBe(400);
    expect(look.boldWeight).toBe(700);
    expect(look.padding).toBe(8);
  });

  it("follows the tokens it is given", () => {
    const tokens: Tokens = {
      ...DEFAULT_TOKENS,
      "terminal.bg": "#ffffff",
      "terminal.palette.4": "#0000ff",
      "font.family": "Iosevka",
      "font.fallback": [],
      "font.size": 16,
      "space.padding": 0,
    };
    const look = terminalAppearance(tokens, "light");
    expect(look.scheme).toBe("light");
    expect(look.background).toEqual({ r: 255, g: 255, b: 255 });
    expect(look.ansi[4]).toEqual({ r: 0, g: 0, b: 255 });
    expect(look.fontFamily).toBe('"Iosevka"');
    expect(look.fontSizePx).toBe(16);
    expect(look.padding).toBe(0);
  });
});
