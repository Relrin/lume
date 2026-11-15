/**
 * Having a terminal's font ready before a terminal is drawn.
 *
 * A terminal's glyphs are drawn once into an atlas and copied from there. A font that
 * the page ships is fetched only when something first uses it, so a terminal drawn at
 * once would fill its atlas, and size its cells, with whatever font stands in until the
 * real one arrives. The faces a terminal draws with are therefore asked for first.
 */

import { FONT_LOAD_TIMEOUT_MS } from "../tuning.ts";
import type { Appearance } from "../theme.ts";

/** The part of a document's fonts this needs: something that loads the faces a font names. */
export interface FontLoader {
  load(font: string, text?: string): Promise<unknown>;
}

/** Any text will do: a face without a range of its own covers every character. */
const SAMPLE = "M";

/** The four faces a terminal draws with under a look, each as CSS names a font. */
export function terminalFaces(appearance: Appearance): string[] {
  const { fontWeight, boldWeight, fontSizePx, fontFamily } = appearance;
  const face = (style: string, weight: number) => `${style}${weight} ${fontSizePx}px ${fontFamily}`;
  return [
    face("", fontWeight),
    face("", boldWeight),
    face("italic ", fontWeight),
    face("italic ", boldWeight),
  ];
}

/**
 * Resolves once the faces a terminal draws with are ready to draw with. A font that is
 * installed on the machine is ready at once; one the page ships is fetched. A face that
 * cannot be had, or takes too long, is not waited for: the terminal is then drawn with
 * the next font in its list, as it would be if the font did not exist.
 */
export async function loadTerminalFonts(
  appearance: Appearance,
  fonts: FontLoader = document.fonts,
): Promise<void> {
  const loading = Promise.all(
    terminalFaces(appearance).map((face) => fonts.load(face, SAMPLE).catch(() => undefined)),
  );
  let timer: ReturnType<typeof setTimeout> | undefined;
  const patience = new Promise<void>((resolve) => {
    timer = setTimeout(resolve, FONT_LOAD_TIMEOUT_MS);
  });
  await Promise.race([loading, patience]);
  clearTimeout(timer);
}
