/**
 * A picture a plugin's thing shows: one of the page's line drawings, by name, or one
 * of the plugin's own pictures.
 *
 * A plugin's picture is drawn only as an image, from the `data:` address the core made
 * of it: an SVG drawn so runs no script and loads nothing. One its manifest says takes
 * the colour around it is drawn as a mask over that colour instead, so it looks as the
 * page's own drawings do. A name the page has no drawing for shows a small dot.
 */

import { Glyph } from "../icons/Glyph.tsx";
import { LINE_GLYPHS, type LineGlyphName } from "../icons/glyphs.generated.ts";
import type { IconRef, PluginImage } from "../core/types.ts";

function isGlyph(name: string): name is LineGlyphName {
  return Object.hasOwn(LINE_GLYPHS, name);
}

export function PluginIcon({
  icon,
  images,
  size = 14,
}: {
  icon: IconRef | undefined;
  images: Readonly<Record<string, PluginImage>> | undefined;
  size?: number;
}) {
  if (icon === undefined) return null;
  if ("glyph" in icon) {
    if (isGlyph(icon.glyph)) return <Glyph name={icon.glyph} size={size} />;
    return <span className="plugin-icon-dot" aria-hidden="true" />;
  }
  const image = images?.[icon.image];
  if (image === undefined) return null;
  if (image.tint) {
    return (
      <span
        className="plugin-icon-mask"
        aria-hidden="true"
        style={{
          width: size,
          height: size,
          maskImage: `url("${image.url}")`,
          WebkitMaskImage: `url("${image.url}")`,
        }}
      />
    );
  }
  return (
    <img
      className="plugin-icon-image"
      src={image.url}
      width={size}
      height={size}
      alt=""
      draggable={false}
    />
  );
}
