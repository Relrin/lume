/**
 * The WebGL2 shaders.
 *
 * Each instance is a rectangle given in device pixels. The four corners come from the
 * vertex number, so no vertex buffer is needed beside the instance records. Colours
 * leave the fragment shader premultiplied by their alpha.
 *
 * Atlas pages are read with `texelFetch`, which takes whole texel coordinates and does
 * no filtering: a glyph is copied to the screen exactly as it was drawn.
 */

const CORNER = `
  vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
  vec2 pixel = vec2(a_position) + corner * vec2(a_size);
  // From pixels with the origin at the top left to clip space.
  gl_Position = vec4(pixel.x / u_surface.x * 2.0 - 1.0, 1.0 - pixel.y / u_surface.y * 2.0, 0.0, 1.0);
`;

export const RECT_VERTEX = `#version 300 es
layout(location = 0) in ivec2 a_position;
layout(location = 1) in uvec2 a_size;
layout(location = 2) in vec4 a_color;
uniform vec2 u_surface;
out vec4 v_color;
void main() {
${CORNER}
  v_color = a_color;
}
`;

export const RECT_FRAGMENT = `#version 300 es
precision highp float;
in vec4 v_color;
out vec4 out_color;
void main() {
  out_color = vec4(v_color.rgb * v_color.a, v_color.a);
}
`;

export const GLYPH_VERTEX = `#version 300 es
layout(location = 0) in ivec2 a_position;
layout(location = 1) in uvec2 a_size;
layout(location = 2) in uvec2 a_texel;
layout(location = 3) in uvec2 a_page;
layout(location = 4) in vec4 a_color;
uniform vec2 u_surface;
out vec2 v_texel;
flat out uvec2 v_page;
out vec4 v_color;
void main() {
${CORNER}
  v_texel = vec2(a_texel) + corner * vec2(a_size);
  v_page = a_page;
  v_color = a_color;
}
`;

export const GLYPH_FRAGMENT = `#version 300 es
precision highp float;
precision highp int;
precision highp sampler2DArray;
uniform sampler2DArray u_mask;
uniform sampler2DArray u_color;
in vec2 v_texel;
flat in uvec2 v_page;
in vec4 v_color;
out vec4 out_color;
void main() {
  ivec3 at = ivec3(ivec2(floor(v_texel)), int(v_page.x));
  if ((v_page.y & 1u) != 0u) {
    // A glyph with its own colours, stored without premultiplication.
    vec4 texel = texelFetch(u_color, at, 0);
    out_color = vec4(texel.rgb * texel.a, texel.a);
  } else {
    // Coverage, tinted with the instance colour.
    float coverage = texelFetch(u_mask, at, 0).r * v_color.a;
    out_color = vec4(v_color.rgb * coverage, coverage);
  }
}
`;
