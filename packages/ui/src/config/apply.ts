/**
 * Putting a configuration into effect on the page.
 *
 * The interface is styled with CSS variables, one for each design token, so applying a
 * theme or a setting is setting variables on the document: nothing is rendered again
 * for it. The variables are set through the style object, which a content policy that
 * forbids inline style attributes still allows.
 */

import { applyCssVariables } from "@lume/tokens";

import type { ConfigProblem, ConfigView } from "../core/types.ts";

/** Sets the configuration's tokens on the document, and tells it the colour scheme. */
export function applyConfig(view: ConfigView, root: HTMLElement = document.documentElement): void {
  applyCssVariables(root, view.tokens);
  // Scrollbars and form controls the page does not draw itself follow the theme.
  root.style.setProperty("color-scheme", view.theme.scheme);
  root.dataset["scheme"] = view.theme.scheme;
  root.dataset["platform"] = view.platform;
}

/** One problem with the user's files, as a line a person can act on. */
export function describeProblem(problem: ConfigProblem): string {
  const file = problem.file === null ? "" : problem.file.replace(/^.*[\\/]/, "");
  const where = file === "" ? "" : problem.line > 0 ? `${file}:${problem.line}: ` : `${file}: `;
  const hint = problem.suggestion === null ? "" : ` (did you mean ${problem.suggestion}?)`;
  return `${where}${problem.message}${hint}`;
}
