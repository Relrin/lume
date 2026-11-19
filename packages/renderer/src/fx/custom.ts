/**
 * Cursor effects drawn by shaders that plugins bring.
 *
 * A plugin writes one function, `vec4 lume_effect(vec2 pixel)`, and any helpers of its
 * own; the application checked it before it reached the page (loops with bounds written
 * as numbers, no textures, no inputs of its own, short). It is put inside the program
 * below, which declares everything the function can read: the canvas's size, how far
 * the effect has got, the cursor's old and new cells and its colour. What it returns is
 * a colour and how much of it covers the pixel; the program premultiplies it, as every
 * effect leaves the canvas.
 *
 * Only the pixels near the cursor's cells are shaded: the rectangle round both, out as
 * far as the effect's manifest says it reaches.
 */

/** A plugin's cursor effect, as the page is given it. */
export interface CustomEffect {
  /** `<plugin>/<name>`. */
  readonly id: string;
  /** The plugin's GLSL. */
  readonly source: string;
  /** How long it lasts, in milliseconds, before the settings' length is applied. */
  readonly duration: number;
  /** How far from the cursor's cells it may draw, in CSS pixels. */
  readonly reach: number;
}

/** What one frame of a plugin's effect is drawn with, in device pixels. */
export interface CustomDraw {
  /** How far the effect has got, from 0 to 1, and the seconds since it began. */
  readonly progress: number;
  readonly time: number;
  /** The cells the cursor left and reached: left, top, width, height. */
  readonly from: readonly [number, number, number, number];
  readonly to: readonly [number, number, number, number];
  /** The cursor's colour, packed. */
  readonly color: number;
  /** The rectangle shaded: left, top, right, bottom. */
  readonly bounds: readonly [number, number, number, number];
  /** A rectangle left clear, where the cursor itself is: left, top, right, bottom. */
  readonly hole: readonly [number, number, number, number];
}

/** The uniforms a plugin's function reads. */
export const CUSTOM_UNIFORMS = [
  "u_surface",
  "u_bounds",
  "lume_resolution",
  "lume_progress",
  "lume_time",
  "lume_from",
  "lume_to",
  "lume_color",
  "lume_hole",
] as const;
export type CustomUniform = (typeof CUSTOM_UNIFORMS)[number];

/** The program a plugin's function is put inside. */
export function customFragment(source: string): string {
  return `#version 300 es
precision highp float;
precision highp int;
uniform vec2 lume_resolution;
uniform float lume_progress;
uniform float lume_time;
uniform vec4 lume_from;
uniform vec4 lume_to;
uniform vec3 lume_color;
uniform vec4 lume_hole;
out vec4 lume_out;
#line 1
${source}
void main() {
  vec2 lume_pixel = vec2(gl_FragCoord.x, lume_resolution.y - gl_FragCoord.y);
  if (lume_pixel.x >= lume_hole.x && lume_pixel.x < lume_hole.z &&
      lume_pixel.y >= lume_hole.y && lume_pixel.y < lume_hole.w) discard;
  vec4 lume_shade = lume_effect(lume_pixel);
  float lume_cover = clamp(lume_shade.a, 0.0, 1.0);
  if (lume_cover <= 0.0) discard;
  lume_out = vec4(clamp(lume_shade.rgb, 0.0, 1.0) * lume_cover, lume_cover);
}
`;
}

/** Why a plugin's shader could not be used. */
export class CustomEffectError extends Error {}
