/**
 * A picture a plugin's thing shows: one of the page's line drawings, by name, or one
 * of the plugin's own pictures.
 *
 * A plugin's picture is drawn only as an image, from the `data:` address the core made
 * of it: an SVG drawn so runs no script and loads nothing. One its manifest says takes
 * the colour around it is drawn as a mask over that colour instead, so it looks as the
 * page's own drawings do. A name the page has no drawing for shows a small dot.
 *
 * The page is told a picture's hash and fetches the picture when it is first drawn;
 * until it is there its room is kept, so nothing moves when it arrives.
 */

import { useEffect, useSyncExternalStore } from "react";

import { Glyph } from "../icons/Glyph.tsx";
import { LINE_GLYPHS, type LineGlyphName } from "../icons/glyphs.generated.ts";
import type { IconRef, PluginImage } from "../core/types.ts";
import { assets } from "./assets.ts";

function isGlyph(name: string): name is LineGlyphName {
  return Object.hasOwn(LINE_GLYPHS, name);
}

/** A plugin's own picture, once it has been fetched. */
function Picture({ image, size }: { image: PluginImage; size: number }) {
  const read = () => assets.peek(image.hash);
  const url = useSyncExternalStore(assets.subscribe, read, read);
  useEffect(() => {
    if (url === undefined) assets.want(image.hash);
  }, [image.hash, url]);
  if (url === undefined) {
    return (
      <span
        className="plugin-icon-wait"
        aria-hidden="true"
        style={{ display: "inline-block", width: size, height: size }}
      />
    );
  }
  if (image.tint) {
    return (
      <span
        className="plugin-icon-mask"
        aria-hidden="true"
        style={{
          width: size,
          height: size,
          maskImage: `url("${url}")`,
          WebkitMaskImage: `url("${url}")`,
        }}
      />
    );
  }
  return (
    <img
      className="plugin-icon-image"
      src={url}
      width={size}
      height={size}
      alt=""
      draggable={false}
    />
  );
}

export function PluginIcon({
  icon,
  images,
  size = 14,
  weight,
}: {
  icon: IconRef | undefined;
  images: Readonly<Record<string, PluginImage>> | undefined;
  size?: number;
  /** How heavy a line drawing's line is; left out, as heavy as the drawings usually are. */
  weight?: number;
}) {
  if (icon === undefined) return null;
  if ("glyph" in icon) {
    if (isGlyph(icon.glyph)) {
      return weight === undefined ? (
        <Glyph name={icon.glyph} size={size} />
      ) : (
        <Glyph name={icon.glyph} size={size} weight={weight} />
      );
    }
    return <span className="plugin-icon-dot" aria-hidden="true" />;
  }
  const image = images?.[icon.image];
  if (image === undefined) return null;
  return <Picture image={image} size={size} />;
}
