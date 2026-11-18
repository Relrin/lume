/**
 * The WebGL2 shaders of the cursor's effects.
 *
 * Each shape is one rectangle, the one it and its glow lie within, so only the pixels
 * near the effect are shaded. The fragment shader says how far each pixel is from the
 * shape (a signed distance: negative inside) and turns that into coverage with a
 * smoothed edge, a glow outside, an optional fade along the trail and a hole where the
 * cursor itself is. Which shape it is comes from a uniform, so every pixel of a draw
 * takes the same branch. Colours leave it premultiplied by their alpha.
 */

import { FX_POLYGON_MAX } from "./tuning.ts";

export const FX_VERTEX = `#version 300 es
uniform vec2 u_surface;
uniform vec4 u_bounds;
void main() {
  vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
  vec2 pixel = mix(u_bounds.xy, u_bounds.zw, corner);
  gl_Position = vec4(pixel.x / u_surface.x * 2.0 - 1.0, 1.0 - pixel.y / u_surface.y * 2.0, 0.0, 1.0);
}
`;

export const FX_FRAGMENT = `#version 300 es
precision highp float;
precision highp int;
#define POLYGON_MAX ${FX_POLYGON_MAX}
uniform vec2 u_surface;
uniform int u_kind;
uniform vec2 u_points[POLYGON_MAX];
uniform int u_count;
uniform vec4 u_box;
uniform float u_radius;
uniform float u_ring;
uniform vec3 u_color;
uniform float u_alpha;
uniform vec4 u_fade;
uniform float u_fadePower;
uniform vec2 u_halo;
uniform vec4 u_hole;
uniform float u_edge;
out vec4 out_color;

// The signed distance to a polygon: the nearest edge gives the distance, and how many
// edges a line from the pixel crosses says whether it is inside.
float polygonDistance(vec2 p) {
  float nearest = dot(p - u_points[0], p - u_points[0]);
  float inside = 1.0;
  for (int i = 0; i < POLYGON_MAX; i++) {
    if (i >= u_count) break;
    int j = i == 0 ? u_count - 1 : i - 1;
    vec2 a = u_points[i];
    vec2 b = u_points[j];
    vec2 edge = b - a;
    vec2 w = p - a;
    vec2 off = w - edge * clamp(dot(w, edge) / dot(edge, edge), 0.0, 1.0);
    nearest = min(nearest, dot(off, off));
    bvec3 crossing = bvec3(p.y >= a.y, p.y < b.y, edge.x * w.y > edge.y * w.x);
    if (all(crossing) || all(not(crossing))) inside = -inside;
  }
  return inside * sqrt(nearest);
}

float boxDistance(vec2 p) {
  vec2 d = abs(p - u_box.xy) - u_box.zw;
  return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

void main() {
  vec2 p = vec2(gl_FragCoord.x, u_surface.y - gl_FragCoord.y);
  if (p.x >= u_hole.x && p.x < u_hole.z && p.y >= u_hole.y && p.y < u_hole.w) discard;
  float d;
  if (u_kind == 0) {
    d = polygonDistance(p);
  } else if (u_kind == 1) {
    d = length(p - u_box.xy) - u_radius;
  } else if (u_kind == 2) {
    d = abs(length(p - u_box.xy) - u_radius) - u_ring * 0.5;
  } else if (u_kind == 3) {
    d = boxDistance(p);
  } else {
    d = abs(boxDistance(p)) - u_ring * 0.5;
  }
  float coverage = 1.0 - smoothstep(-u_edge, u_edge, d);
  if (u_halo.x > 0.0) coverage = max(coverage, u_halo.y * exp(-max(d, 0.0) / u_halo.x));
  if (u_fadePower > 0.0) {
    vec2 along = u_fade.zw - u_fade.xy;
    float length2 = dot(along, along);
    float f = length2 > 0.0 ? clamp(dot(p - u_fade.xy, along) / length2, 0.0, 1.0) : 1.0;
    coverage *= pow(f, u_fadePower);
  }
  float alpha = coverage * u_alpha;
  if (alpha <= 0.0) discard;
  out_color = vec4(u_color * alpha, alpha);
}
`;

/** The uniforms the fragment shader reads, by name. */
export const FX_UNIFORMS = [
  "u_surface",
  "u_bounds",
  "u_kind",
  "u_points",
  "u_count",
  "u_box",
  "u_radius",
  "u_ring",
  "u_color",
  "u_alpha",
  "u_fade",
  "u_fadePower",
  "u_halo",
  "u_hole",
  "u_edge",
] as const;
export type FxUniform = (typeof FX_UNIFORMS)[number];
