/**
 * The WebGL2 tier.
 *
 * Each plane of instance records is one vertex buffer drawn with one instanced call.
 * Atlas pages are layers of two array textures, one for coverage masks and one for
 * colour glyphs. Every frame redraws everything from the buffers, which a GPU does in
 * well under a millisecond; only the bytes of changed rows are sent to it.
 *
 * WebGL contexts cannot share textures, so each pane has its own atlas.
 */

import type { GlyphAtlas } from "../atlas/atlas.ts";
import { ATLAS_PAGE_SIZE, WEBGL_FAIL_IF_MAJOR_PERFORMANCE_CAVEAT } from "../constants.ts";
import { GLYPH, PLANE_ORDER, type PlaneName, type Planes, type RecordLayout } from "../layout.ts";
import { TierUnavailableError } from "../select.ts";
import type { AtlasSink, PageKind, PresentInfo, Tier, TierBackend } from "../types.ts";
import { attributePlan, type ComponentType } from "./plan.ts";
import { GLYPH_FRAGMENT, GLYPH_VERTEX, RECT_FRAGMENT, RECT_VERTEX } from "./shaders.ts";

type GL = WebGL2RenderingContext;

function compile(gl: GL, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) throw new TierUnavailableError("webgl2", "could not create a shader");
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS) && !gl.isContextLost()) {
    throw new Error(`a WebGL2 shader failed to compile: ${gl.getShaderInfoLog(shader) ?? ""}`);
  }
  return shader;
}

function link(gl: GL, vertex: string, fragment: string): WebGLProgram {
  const program = gl.createProgram();
  if (!program) throw new TierUnavailableError("webgl2", "could not create a program");
  gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, vertex));
  gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, fragment));
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS) && !gl.isContextLost()) {
    throw new Error(`a WebGL2 program failed to link: ${gl.getProgramInfoLog(program) ?? ""}`);
  }
  return program;
}

/** Atlas pages as layers of array textures. */
class TexturePages implements AtlasSink {
  readonly #gl: GL;
  readonly textures: Record<PageKind, WebGLTexture | undefined> = {
    mask: undefined,
    color: undefined,
  };
  readonly #layers: Record<PageKind, number> = { mask: 0, color: 0 };

  constructor(gl: GL) {
    this.#gl = gl;
  }

  ensurePages(kind: PageKind, count: number): void {
    if (count <= this.#layers[kind]) return;
    const gl = this.#gl;
    // An array texture cannot gain layers, so a larger one is made and the pages
    // already drawn are copied across.
    const texture = this.#create(kind, count);
    const old = this.textures[kind];
    if (old) {
      const framebuffer = gl.createFramebuffer();
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, framebuffer);
      gl.bindTexture(gl.TEXTURE_2D_ARRAY, texture);
      for (let layer = 0; layer < this.#layers[kind]; layer++) {
        gl.framebufferTextureLayer(gl.READ_FRAMEBUFFER, gl.COLOR_ATTACHMENT0, old, 0, layer);
        gl.copyTexSubImage3D(
          gl.TEXTURE_2D_ARRAY,
          0,
          0,
          0,
          layer,
          0,
          0,
          ATLAS_PAGE_SIZE,
          ATLAS_PAGE_SIZE,
        );
      }
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, null);
      gl.deleteFramebuffer(framebuffer);
      gl.deleteTexture(old);
    }
    this.textures[kind] = texture;
    this.#layers[kind] = count;
  }

  upload(
    kind: PageKind,
    page: number,
    x: number,
    y: number,
    width: number,
    height: number,
    pixels: Uint8Array,
  ): void {
    const gl = this.#gl;
    const texture = this.textures[kind];
    if (!texture) return;
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, texture);
    // Rows of single bytes are not padded to four.
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texSubImage3D(
      gl.TEXTURE_2D_ARRAY,
      0,
      x,
      y,
      page,
      width,
      height,
      1,
      kind === "mask" ? gl.RED : gl.RGBA,
      gl.UNSIGNED_BYTE,
      pixels,
    );
  }

  /** A texture to bind where a kind has no pages yet, so the sampler has something. */
  placeholder(kind: PageKind): WebGLTexture {
    return (this.textures[kind] ??= this.#create(kind, 1, 1));
  }

  dispose(): void {
    for (const kind of ["mask", "color"] as const) {
      if (this.textures[kind]) this.#gl.deleteTexture(this.textures[kind]);
      this.textures[kind] = undefined;
      this.#layers[kind] = 0;
    }
  }

  #create(kind: PageKind, layers: number, size = ATLAS_PAGE_SIZE): WebGLTexture {
    const gl = this.#gl;
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, texture);
    gl.texStorage3D(gl.TEXTURE_2D_ARRAY, 1, kind === "mask" ? gl.R8 : gl.RGBA8, size, size, layers);
    // Without these a texture with one mip level counts as incomplete and reads black.
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return texture;
  }
}

interface PlaneState {
  buffer: WebGLBuffer;
  vertexArray: WebGLVertexArrayObject;
  layout: RecordLayout;
  capacity: number;
}

/** Draws instance records with WebGL2. */
export class WebGl2Backend implements TierBackend {
  readonly tier: Tier = "webgl2";
  readonly #canvas: HTMLCanvasElement;
  readonly #gl: GL;
  readonly #rect: WebGLProgram;
  readonly #glyph: WebGLProgram;
  readonly #rectSurface: WebGLUniformLocation | null;
  readonly #glyphSurface: WebGLUniformLocation | null;
  #pages: TexturePages;
  #atlasKey: string | undefined;
  #atlas: GlyphAtlas | undefined;
  #planes: Planes | undefined;
  #state: Partial<Record<PlaneName, PlaneState>> = {};
  #lost: ((reason: string) => void) | undefined;
  #disposed = false;
  readonly #onContextLost = () => this.#lost?.("the WebGL context was lost");

  constructor(canvas: HTMLCanvasElement) {
    this.#canvas = canvas;
    const gl = canvas.getContext("webgl2", {
      alpha: false,
      antialias: false,
      depth: false,
      stencil: false,
      premultipliedAlpha: true,
      preserveDrawingBuffer: false,
      powerPreference: "low-power",
      failIfMajorPerformanceCaveat: WEBGL_FAIL_IF_MAJOR_PERFORMANCE_CAVEAT,
    });
    if (!gl) throw new TierUnavailableError("webgl2", "no WebGL2 context");
    this.#gl = gl;
    canvas.addEventListener("webglcontextlost", this.#onContextLost);

    this.#rect = link(gl, RECT_VERTEX, RECT_FRAGMENT);
    this.#glyph = link(gl, GLYPH_VERTEX, GLYPH_FRAGMENT);
    this.#rectSurface = gl.getUniformLocation(this.#rect, "u_surface");
    this.#glyphSurface = gl.getUniformLocation(this.#glyph, "u_surface");
    gl.useProgram(this.#glyph);
    gl.uniform1i(gl.getUniformLocation(this.#glyph, "u_mask"), 0);
    gl.uniform1i(gl.getUniformLocation(this.#glyph, "u_color"), 1);

    this.#pages = new TexturePages(gl);
    gl.disable(gl.DEPTH_TEST);
    gl.enable(gl.BLEND);
    // Colours arrive premultiplied.
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
  }

  atlas(key: string, create: (sink: AtlasSink) => GlyphAtlas): GlyphAtlas {
    if (this.#atlas && this.#atlasKey === key) return this.#atlas;
    this.#pages.dispose();
    this.#pages = new TexturePages(this.#gl);
    this.#atlas = create(this.#pages);
    this.#atlasKey = key;
    return this.#atlas;
  }

  configure(width: number, height: number): void {
    this.#canvas.width = width;
    this.#canvas.height = height;
  }

  setPlanes(planes: Planes): void {
    const gl = this.#gl;
    this.#deletePlanes();
    this.#planes = planes;
    for (const name of PLANE_ORDER) {
      const plane = planes[name];
      const buffer = gl.createBuffer();
      const vertexArray = gl.createVertexArray();
      gl.bindVertexArray(vertexArray);
      gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
      gl.bufferData(gl.ARRAY_BUFFER, plane.bytes.byteLength, gl.DYNAMIC_DRAW);

      const types: Record<ComponentType, number> = {
        short: gl.SHORT,
        "unsigned-short": gl.UNSIGNED_SHORT,
        "unsigned-byte": gl.UNSIGNED_BYTE,
      };
      const { stride, attributes } = attributePlan(plane.layout);
      for (const attribute of attributes) {
        gl.enableVertexAttribArray(attribute.location);
        const type = types[attribute.type];
        if (attribute.integer) {
          gl.vertexAttribIPointer(
            attribute.location,
            attribute.components,
            type,
            stride,
            attribute.offset,
          );
        } else {
          gl.vertexAttribPointer(
            attribute.location,
            attribute.components,
            type,
            true,
            stride,
            attribute.offset,
          );
        }
        // One record per instance, not per corner.
        gl.vertexAttribDivisor(attribute.location, 1);
      }
      this.#state[name] = { buffer, vertexArray, layout: plane.layout, capacity: plane.capacity };
    }
    gl.bindVertexArray(null);
  }

  upload(name: PlaneName, byteStart: number, byteEnd: number): void {
    const state = this.#state[name];
    const plane = this.#planes?.[name];
    if (!state || !plane || byteEnd <= byteStart) return;
    const gl = this.#gl;
    gl.bindBuffer(gl.ARRAY_BUFFER, state.buffer);
    gl.bufferSubData(gl.ARRAY_BUFFER, byteStart, plane.bytes, byteStart, byteEnd - byteStart);
  }

  present(info: PresentInfo): void {
    const gl = this.#gl;
    if (gl.isContextLost()) return;
    gl.viewport(0, 0, info.width, info.height);
    const clear = info.clear;
    gl.clearColor(
      (clear & 0xff) / 255,
      ((clear >>> 8) & 0xff) / 255,
      ((clear >>> 16) & 0xff) / 255,
      1,
    );
    gl.clear(gl.COLOR_BUFFER_BIT);

    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, this.#pages.placeholder("mask"));
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, this.#pages.placeholder("color"));

    let current: WebGLProgram | undefined;
    for (const name of PLANE_ORDER) {
      const state = this.#state[name];
      if (!state) continue;
      const count =
        name === "overlayRect"
          ? info.overlayRects
          : name === "overlayGlyph"
            ? info.overlayGlyphs
            : Math.min(info.used[name], state.capacity);
      if (count === 0) continue;
      const glyphs = state.layout === GLYPH;
      const program = glyphs ? this.#glyph : this.#rect;
      if (program !== current) {
        current = program;
        gl.useProgram(program);
        gl.uniform2f(glyphs ? this.#glyphSurface : this.#rectSurface, info.width, info.height);
      }
      gl.bindVertexArray(state.vertexArray);
      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, count);
    }
    gl.bindVertexArray(null);
  }

  onLost(listener: (reason: string) => void): void {
    this.#lost = listener;
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#canvas.removeEventListener("webglcontextlost", this.#onContextLost);
    this.#lost = undefined;
    const gl = this.#gl;
    this.#deletePlanes();
    this.#pages.dispose();
    gl.deleteProgram(this.#rect);
    gl.deleteProgram(this.#glyph);
    // A browser allows only a few live contexts. Giving this one up at once, instead
    // of when the canvas is collected, leaves room for the pane that replaces it.
    gl.getExtension("WEBGL_lose_context")?.loseContext();
  }

  #deletePlanes(): void {
    const gl = this.#gl;
    for (const state of Object.values(this.#state)) {
      gl.deleteBuffer(state.buffer);
      gl.deleteVertexArray(state.vertexArray);
    }
    this.#state = {};
  }
}
