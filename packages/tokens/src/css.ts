/**
 * Tokens as CSS variables.
 *
 * The interface is styled with CSS variables and nothing else, one variable for each
 * token. The application sends the tokens' values; this turns each into the text its
 * variable holds and sets it. Setting goes through the style object, which a content
 * policy that forbids inline style attributes still allows.
 */

import {
  CSS_VARIABLES,
  TOKEN_KINDS,
  TOKEN_NAMES,
  type TokenName,
  type Tokens,
} from "./generated.ts";

/** The font names CSS knows without quotes. */
const GENERIC_FAMILIES: ReadonlySet<string> = new Set([
  "serif",
  "sans-serif",
  "monospace",
  "cursive",
  "fantasy",
  "system-ui",
  "ui-serif",
  "ui-sans-serif",
  "ui-monospace",
  "ui-rounded",
  "emoji",
  "math",
  "fangsong",
]);

/** A string inside double quotes, as CSS writes one. */
function quoted(text: string): string {
  return `"${text.replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`;
}

/** A font's name as CSS writes it: quoted, unless it is one of CSS's own names. */
export function cssFamily(name: string): string {
  return GENERIC_FAMILIES.has(name) ? name : quoted(name);
}

/**
 * A font and the fonts to fall back on, as one CSS `font-family` value, most preferred
 * first.
 */
export function cssFontStack(family: string, fallback: readonly string[]): string {
  return [family, ...fallback].map(cssFamily).join(", ");
}

/** The value of a token as its CSS variable holds it. */
export function cssValue<Name extends TokenName>(name: Name, value: Tokens[Name]): string {
  // The kind says how the value is written; the checks narrow what the type cannot.
  const held: string | number | readonly string[] | null = value;
  switch (TOKEN_KINDS[name]) {
    case "length":
      return `${String(held)}px`;
    case "number":
      return String(held);
    case "font-family":
      return cssFamily(String(held));
    case "font-list":
      return Array.isArray(held) ? held.map(cssFamily).join(", ") : "";
    case "image":
      return typeof held === "string" && held !== "" ? `url(${quoted(held)})` : "none";
    case "duration":
      return `${String(held)}ms`;
    case "color":
    case "shadow":
    case "easing":
      return String(held);
  }
}

/** Something whose style can be set: in a page, an element. */
export interface Styled {
  readonly style: { setProperty(name: string, value: string): void };
}

/**
 * Sets every token as a CSS variable on `target`. Applied to the document's root
 * element, the whole page follows.
 */
export function applyCssVariables(target: Styled, tokens: Tokens): void {
  for (const name of TOKEN_NAMES) {
    target.style.setProperty(CSS_VARIABLES[name], cssValue(name, tokens[name]));
  }
}
