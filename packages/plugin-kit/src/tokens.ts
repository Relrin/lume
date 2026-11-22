/**
 * The window's look inside a view.
 *
 * A view is told the window's design tokens when it starts and whenever they change: a
 * CSS variable and a value for every colour, size and font the window is drawn with
 * (`--lume-color-surface`, `--lume-radius-control`, …). Setting them on the view's root
 * element is all it takes for styles written with `var(--lume-…)` to follow the theme.
 */

/** The window's tokens: each CSS variable and its value. */
export type ThemeTokens = Readonly<Record<string, string>>;

/** The part of an element the tokens are set on. */
export interface Styleable {
  readonly style: { setProperty(name: string, value: string): void };
  setAttribute?(name: string, value: string): void;
}

/** Whether a name is one of the window's variables, which are the only ones set. */
function ours(name: string): boolean {
  return /^--lume-[a-z0-9-]{1,96}$/.test(name);
}

/**
 * Sets the tokens on `root` as CSS variables, and says there whether the look is light
 * or dark (`data-scheme`, and `color-scheme` for the browser's own controls).
 */
export function applyTokens(tokens: ThemeTokens, scheme: string, root?: Styleable): void {
  const target = root ?? globalThis.document?.documentElement;
  if (!target) return;
  for (const [name, value] of Object.entries(tokens)) {
    if (ours(name) && typeof value === "string") target.style.setProperty(name, value);
  }
  const plain = scheme === "light" ? "light" : "dark";
  target.style.setProperty("color-scheme", plain);
  target.setAttribute?.("data-scheme", plain);
}

/**
 * Says on `root` where the view is shown (`data-place`), for styles that differ by it:
 * a view on a page of the settings window is drawn on the window's own ground.
 */
export function applyPlace(place: string, root?: Styleable): void {
  const target = root ?? globalThis.document?.documentElement;
  target?.setAttribute?.("data-place", place);
}
