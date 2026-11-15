import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import {
  applyCssVariables,
  CSS_VARIABLES,
  cssFamily,
  cssFontStack,
  cssValue,
  DEFAULT_TOKENS,
  TOKEN_KINDS,
  TOKEN_NAMES,
  type Tokens,
} from "../src/index.ts";

/** What `applyCssVariables` sets, by variable. */
function applied(tokens: Tokens): Map<string, string> {
  const set = new Map<string, string>();
  applyCssVariables({ style: { setProperty: (name, value) => set.set(name, value) } }, tokens);
  return set;
}

/** The variables a stylesheet sets on the root, by variable. */
function declared(css: string): Map<string, string> {
  const set = new Map<string, string>();
  for (const line of css.split("\n")) {
    const match = /^\s*(--[a-z0-9-]+): (.*);$/.exec(line);
    if (match?.[1] !== undefined && match[2] !== undefined) set.set(match[1], match[2]);
  }
  return set;
}

describe("the token set", () => {
  it("gives every token a kind and a CSS variable of its own", () => {
    expect(TOKEN_NAMES.length).toBeGreaterThan(60);
    expect(Object.keys(CSS_VARIABLES)).toEqual([...TOKEN_NAMES]);
    expect(new Set(Object.values(CSS_VARIABLES)).size).toBe(TOKEN_NAMES.length);
    expect(CSS_VARIABLES["color.status.info"]).toBe("--lume-color-status-info");
    expect(TOKEN_KINDS["terminal.palette.15"]).toBe("color");
    expect(Object.keys(DEFAULT_TOKENS)).toEqual([...TOKEN_NAMES]);
  });
});

describe("a token's value as CSS", () => {
  it("writes each kind the way CSS takes it", () => {
    expect(cssValue("color.surface", "#0e1015")).toBe("#0e1015");
    expect(cssValue("color.overlay", "#00000073")).toBe("#00000073");
    expect(cssValue("space.gap", 8)).toBe("8px");
    expect(cssValue("radius.card", 0.5)).toBe("0.5px");
    expect(cssValue("font.line-height", 1.2)).toBe("1.2");
    expect(cssValue("font.weight", 400)).toBe("400");
    expect(cssValue("shadow.card", "0px 1px 3px 0px #00000040")).toBe("0px 1px 3px 0px #00000040");
  });

  it("quotes a font's name unless it is one of CSS's own", () => {
    expect(cssFamily("JetBrains Mono")).toBe('"JetBrains Mono"');
    expect(cssFamily("monospace")).toBe("monospace");
    expect(cssFamily('A "B" C')).toBe('"A \\"B\\" C"');
    expect(cssValue("font.family", "JetBrains Mono")).toBe('"JetBrains Mono"');
    expect(cssValue("font.ui-family", "system-ui")).toBe("system-ui");
    expect(cssValue("font.fallback", ["Cascadia Mono", "monospace"])).toBe(
      '"Cascadia Mono", monospace',
    );
    expect(cssFontStack("Iosevka", ["Menlo", "monospace"])).toBe('"Iosevka", "Menlo", monospace');
  });

  it("writes a picture as a URL and no picture as none", () => {
    expect(cssValue("background.image", null)).toBe("none");
    expect(cssValue("background.image", "")).toBe("none");
    expect(cssValue("background.image", 'C:\\pics\\a "b".png')).toBe(
      'url("C:\\\\pics\\\\a \\"b\\".png")',
    );
  });
});

describe("applying tokens", () => {
  it("sets one variable for each token", () => {
    const set = applied(DEFAULT_TOKENS);
    expect(set.size).toBe(TOKEN_NAMES.length);
    expect(set.get("--lume-color-surface")).toBe("#121416");
    expect(set.get("--lume-font-size")).toBe("14px");
    expect(set.get("--lume-background-image")).toBe("none");
  });

  it("sets exactly what the stylesheet a page starts from declares", () => {
    const css = readFileSync(new URL("../src/defaults.css", import.meta.url), "utf8");
    expect(applied(DEFAULT_TOKENS)).toEqual(declared(css.replaceAll("\r\n", "\n")));
  });

  it("follows a change of the tokens", () => {
    const changed: Tokens = { ...DEFAULT_TOKENS, "space.gap": 2, "color.accent": "#ff8800" };
    const set = applied(changed);
    expect(set.get("--lume-space-gap")).toBe("2px");
    expect(set.get("--lume-color-accent")).toBe("#ff8800");
    expect(set.get("--lume-radius-card")).toBe("10px");
  });
});
