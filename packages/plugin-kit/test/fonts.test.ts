import { afterEach, describe, expect, it, vi } from "vitest";

import { addWindowFonts, forgetWindowFonts, WINDOW_FONTS } from "../src/fonts";

interface Declared {
  readonly family: string;
  readonly source: string;
  readonly weight: string | undefined;
  readonly style: string | undefined;
  readonly display: string | undefined;
}

function withFontFace(): void {
  vi.stubGlobal(
    "FontFace",
    class {
      readonly declared: Declared;
      constructor(family: string, source: string, descriptors?: FontFaceDescriptors) {
        this.declared = {
          family,
          source,
          weight: descriptors?.weight,
          style: descriptors?.style,
          display: descriptors?.display,
        };
      }
    },
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  forgetWindowFonts();
});

describe("the window's fonts in a view", () => {
  it("are declared once, from the place the application serves them", () => {
    withFontFace();
    const faces: { declared: Declared }[] = [];
    const page = { fonts: { add: (face: FontFace) => faces.push(face as never) } };
    addWindowFonts(page);
    addWindowFonts(page);
    expect(faces).toHaveLength(WINDOW_FONTS.length);
    expect(faces.map((face) => face.declared.family)).toEqual([
      "Inter",
      "Inter",
      "JetBrains Mono",
      "JetBrains Mono",
      "JetBrains Mono",
      "JetBrains Mono",
    ]);
    expect(faces[0]?.declared).toEqual({
      family: "Inter",
      source: 'url("/lume/fonts/InterVariable.woff2") format("woff2")',
      weight: "100 900",
      style: "normal",
      display: "block",
    });
    expect(faces[1]?.declared.style).toBe("italic");
    for (const face of faces)
      expect(face.declared.source).toMatch(/^url\("\/lume\/fonts\/[\w-]+\.woff2"\)/);
  });

  it("are done without where a page cannot declare fonts", () => {
    const add = vi.fn();
    // No `FontFace` here at all.
    expect(() => addWindowFonts({ fonts: { add } })).not.toThrow();
    expect(add).not.toHaveBeenCalled();
    withFontFace();
    expect(() => addWindowFonts({})).not.toThrow();
    expect(() => addWindowFonts(undefined)).not.toThrow();
  });
});
