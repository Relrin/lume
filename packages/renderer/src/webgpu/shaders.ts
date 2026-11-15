/**
 * The WebGPU shaders.
 *
 * Each instance is a rectangle given in device pixels. The four corners come from the
 * vertex number, so no vertex buffer is needed beside the instance records. Colours
 * leave the fragment shader premultiplied by their alpha.
 *
 * Atlas pages are read with `textureLoad`, which takes whole texel coordinates and does
 * no filtering: a glyph is copied to the screen exactly as it was drawn.
 */

export const SHADER = /* wgsl */ `
struct Surface {
  size: vec2f,
}

@group(0) @binding(0) var<uniform> surface: Surface;
@group(0) @binding(1) var mask_pages: texture_2d_array<f32>;
@group(0) @binding(2) var color_pages: texture_2d_array<f32>;

fn corner_of(vertex: u32) -> vec2f {
  return vec2f(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
}

// From pixels with the origin at the top left to clip space.
fn to_clip(pixel: vec2f) -> vec4f {
  return vec4f(
    pixel.x / surface.size.x * 2.0 - 1.0,
    1.0 - pixel.y / surface.size.y * 2.0,
    0.0,
    1.0,
  );
}

struct RectVertex {
  @builtin(position) position: vec4f,
  @location(0) color: vec4f,
}

@vertex
fn rect_vertex(
  @builtin(vertex_index) vertex: u32,
  @location(0) position: vec2i,
  @location(1) size: vec2u,
  @location(2) color: vec4f,
) -> RectVertex {
  var out: RectVertex;
  out.position = to_clip(vec2f(position) + corner_of(vertex) * vec2f(size));
  out.color = color;
  return out;
}

@fragment
fn rect_fragment(in: RectVertex) -> @location(0) vec4f {
  return vec4f(in.color.rgb * in.color.a, in.color.a);
}

struct GlyphVertex {
  @builtin(position) position: vec4f,
  @location(0) texel: vec2f,
  @location(1) @interpolate(flat) page: vec2u,
  @location(2) color: vec4f,
}

@vertex
fn glyph_vertex(
  @builtin(vertex_index) vertex: u32,
  @location(0) position: vec2i,
  @location(1) size: vec2u,
  @location(2) texel: vec2u,
  @location(3) page: vec2u,
  @location(4) color: vec4f,
) -> GlyphVertex {
  let corner = corner_of(vertex);
  var out: GlyphVertex;
  out.position = to_clip(vec2f(position) + corner * vec2f(size));
  out.texel = vec2f(texel) + corner * vec2f(size);
  out.page = page;
  out.color = color;
  return out;
}

@fragment
fn glyph_fragment(in: GlyphVertex) -> @location(0) vec4f {
  let at = vec2i(floor(in.texel));
  let layer = i32(in.page.x);
  if ((in.page.y & 1u) != 0u) {
    // A glyph with its own colours, stored without premultiplication.
    let texel = textureLoad(color_pages, at, layer, 0);
    return vec4f(texel.rgb * texel.a, texel.a);
  }
  // Coverage, tinted with the instance colour.
  let coverage = textureLoad(mask_pages, at, layer, 0).r * in.color.a;
  return vec4f(in.color.rgb * coverage, coverage);
}
`;
