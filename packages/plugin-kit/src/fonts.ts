/**
 * The window's fonts inside a view.
 *
 * A view may load fonts from its own package and from one other place: the application
 * serves the fonts it ships, the interface's and the terminal's, to every view. Their
 * faces are declared here under the names the window's tokens use
 * (`--lume-font-ui-family`, `--lume-font-family`), so text styled with those is drawn
 * as the window draws it, on a machine that has the fonts installed or not. A face is
 * fetched when something first uses it.
 */

/** Where the application serves its fonts, as a view's page sees it. */
export const WINDOW_FONTS_PATH = "/lume/fonts/";

interface Face {
  readonly family: string;
  readonly file: string;
  readonly weight: string;
  readonly style: "normal" | "italic";
}

/** The faces the application ships. */
export const WINDOW_FONTS: readonly Face[] = [
  { family: "Inter", file: "InterVariable.woff2", weight: "100 900", style: "normal" },
  { family: "Inter", file: "InterVariable-Italic.woff2", weight: "100 900", style: "italic" },
  { family: "JetBrains Mono", file: "JetBrainsMono-Regular.woff2", weight: "400", style: "normal" },
  { family: "JetBrains Mono", file: "JetBrainsMono-Bold.woff2", weight: "700", style: "normal" },
  { family: "JetBrains Mono", file: "JetBrainsMono-Italic.woff2", weight: "400", style: "italic" },
  {
    family: "JetBrains Mono",
    file: "JetBrainsMono-BoldItalic.woff2",
    weight: "700",
    style: "italic",
  },
];

/** The part of a page the faces are added to. */
export interface FontTarget {
  readonly fonts?: { add(face: FontFace): unknown } | undefined;
}

let added = false;

/**
 * Declares the window's fonts on the page. Done once, however often it is asked for;
 * where a page cannot declare fonts this way, nothing is done and text is drawn in the
 * fonts the machine has.
 */
export function addWindowFonts(page: FontTarget | undefined = globalThis.document): void {
  if (added || typeof FontFace !== "function" || !page?.fonts) return;
  added = true;
  for (const face of WINDOW_FONTS) {
    try {
      page.fonts.add(
        new FontFace(face.family, `url("${WINDOW_FONTS_PATH}${face.file}") format("woff2")`, {
          weight: face.weight,
          style: face.style,
          // Text waits a moment for the face rather than being drawn in another first.
          display: "block",
        }),
      );
    } catch {
      // A face that cannot be declared is done without.
    }
  }
}

/** For tests: lets the fonts be declared again. */
export function forgetWindowFonts(): void {
  added = false;
}
