/**
 * Moving the keyboard from a terminal to the bar along the top of the window.
 *
 * A terminal keeps the Tab key for its program, so the window's own controls cannot be
 * reached from it by tabbing. This puts the keyboard on the first of them; from there
 * Tab and the arrow keys go through the rest, and Escape goes back to the terminal.
 */

/** The bar along the top of the window. */
const BAR = ".top-bar";

/** What the Tab key stops at. */
const TABBABLE = [
  "button:not(:disabled):not([tabindex='-1'])",
  "input:not(:disabled):not([tabindex='-1'])",
  "select:not(:disabled):not([tabindex='-1'])",
  "a[href]:not([tabindex='-1'])",
  "[tabindex]:not([tabindex='-1'])",
].join(", ");

/**
 * Puts the keyboard on the first control of the window's top bar that the Tab key
 * stops at. Does nothing in a window that has no such control.
 */
export function focusBar(within: ParentNode = document): void {
  within.querySelector(BAR)?.querySelector<HTMLElement>(TABBABLE)?.focus();
}
