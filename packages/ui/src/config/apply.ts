/**
 * Putting a configuration into effect on the page.
 *
 * The interface is styled with CSS variables, one for each design token, so applying a
 * theme or a setting is setting variables on the document: nothing is rendered again
 * for it. The variables are set through the style object, which a content policy that
 * forbids inline style attributes still allows.
 *
 * While the system asks for less motion every length of motion is nought, whatever
 * the theme says; the page follows the system's wish as it changes.
 */

import { applyCssVariables, CSS_VARIABLES, TOKEN_KINDS, type TokenName } from "@lume/tokens";

import type { ConfigProblem, ConfigView } from "../core/types.ts";

/** The tokens that say how long motion takes. */
const DURATIONS = (Object.keys(TOKEN_KINDS) as TokenName[]).filter(
  (name) => TOKEN_KINDS[name] === "duration",
);

/** What was last applied, to apply again when the system's wish for motion changes. */
let last: { readonly view: ConfigView; readonly root: HTMLElement } | null = null;
let following = false;

function lessMotion(): MediaQueryList | undefined {
  return typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-reduced-motion: reduce)")
    : undefined;
}

/** Sets the configuration's tokens on the document, and tells it the colour scheme. */
export function applyConfig(view: ConfigView, root: HTMLElement = document.documentElement): void {
  applyCssVariables(root, view.tokens);
  const less = lessMotion();
  if (less?.matches === true) {
    for (const name of DURATIONS) root.style.setProperty(CSS_VARIABLES[name], "0ms");
  }
  // Scrollbars and form controls the page does not draw itself follow the theme.
  root.style.setProperty("color-scheme", view.theme.scheme);
  root.dataset["scheme"] = view.theme.scheme;
  root.dataset["platform"] = view.platform;
  last = { view, root };
  if (!following && less !== undefined) {
    following = true;
    less.addEventListener("change", () => {
      if (last) applyConfig(last.view, last.root);
    });
  }
}

/** One problem with the user's files, as a line a person can act on. */
export function describeProblem(problem: ConfigProblem): string {
  const file = problem.file === null ? "" : problem.file.replace(/^.*[\\/]/, "");
  const where = file === "" ? "" : problem.line > 0 ? `${file}:${problem.line}: ` : `${file}: `;
  const hint = problem.suggestion === null ? "" : ` (did you mean ${problem.suggestion}?)`;
  return `${where}${problem.message}${hint}`;
}
