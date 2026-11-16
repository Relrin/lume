/**
 * The design tokens: every colour, size and font the interface is drawn with.
 *
 * The set is defined once, in the application's configuration, and the names, kinds
 * and CSS variables here are generated from that definition. The application resolves
 * a theme and the user's settings to one value for every token and sends them; this
 * package turns them into CSS variables for the interface and into the numbers a
 * terminal is drawn with.
 */

export { applyCssVariables, cssFamily, cssFontStack, cssValue, type Styled } from "./css.ts";
export {
  CSS_VARIABLES,
  DEFAULT_SCHEME,
  DEFAULT_TOKENS,
  LIGHT_TOKENS,
  TOKEN_KINDS,
  TOKEN_NAMES,
  type TokenKind,
  type TokenName,
  type Tokens,
} from "./generated.ts";
export {
  parseColor,
  terminalAppearance,
  type Rgb,
  type Rgba,
  type Scheme,
  type TerminalAppearance,
} from "./terminal.ts";
