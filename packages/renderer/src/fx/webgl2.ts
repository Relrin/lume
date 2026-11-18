/**
 * Drawing the cursor's effects with WebGL2.
 *
 * One small program, one draw per shape and no buffers at all: the four corners of
 * each shape's rectangle come from the vertex number, and everything else is a
 * uniform. The program is compiled when the effects are turned on, and where the
 * browser compiles in the background it is not waited for until the first effect is
 * drawn, so turning them on never stalls a frame.
 */

import { WEBGL_FAIL_IF_MAJOR_PERFORMANCE_CAVEAT } from "../constants.ts";
import { channels, type FxBackend, type FxShape } from "./shape.ts";
import { FX_FRAGMENT, FX_UNIFORMS, FX_VERTEX, type FxUniform } from "./shaders.ts";
import { FX_EDGE } from "./tuning.ts";

type GL = WebGL2RenderingContext;

/** Thrown when a canvas cannot have a WebGL2 context for the effects. */
export class FxUnavailableError extends Error {}

function compile(gl: GL, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) throw new FxUnavailableError("could not create a shader");
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  return shader;
}

export class WebGl2Fx implements FxBackend {
  readonly kind = "webgl2" as const;
  readonly #canvas: HTMLCanvasElement;
  readonly #gl: GL;
  readonly #program: WebGLProgram;
  readonly #shaders: readonly WebGLShader[];
  readonly #at = {} as Record<FxUniform, WebGLUniformLocation | null>;
  #checked = false;
  #lost: (() => void) | undefined;
  #disposed = false;
  readonly #onContextLost = (event: Event) => {
    event.preventDefault();
    this.#lost?.();
  };

  constructor(canvas: HTMLCanvasElement) {
    this.#canvas = canvas;
    const gl = canvas.getContext("webgl2", {
      alpha: true,
      premultipliedAlpha: true,
      antialias: false,
      depth: false,
      stencil: false,
      preserveDrawingBuffer: false,
      powerPreference: "low-power",
      failIfMajorPerformanceCaveat: WEBGL_FAIL_IF_MAJOR_PERFORMANCE_CAVEAT,
    });
    if (!gl) throw new FxUnavailableError("no WebGL2 context");
    this.#gl = gl;
    canvas.addEventListener("webglcontextlost", this.#onContextLost);
    // Asked for so that the program below is compiled off the main thread.
    gl.getExtension("KHR_parallel_shader_compile");

    const program = gl.createProgram();
    if (!program) throw new FxUnavailableError("could not create a program");
    this.#shaders = [
      compile(gl, gl.VERTEX_SHADER, FX_VERTEX),
      compile(gl, gl.FRAGMENT_SHADER, FX_FRAGMENT),
    ];
    for (const shader of this.#shaders) gl.attachShader(program, shader);
    gl.linkProgram(program);
    this.#program = program;

    gl.disable(gl.DEPTH_TEST);
    gl.enable(gl.BLEND);
    // Colours arrive premultiplied.
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.clearColor(0, 0, 0, 0);
  }

  resize(width: number, height: number): void {
    if (this.#canvas.width !== width) this.#canvas.width = width;
    if (this.#canvas.height !== height) this.#canvas.height = height;
  }

  clear(): void {
    const gl = this.#gl;
    gl.viewport(0, 0, this.#canvas.width, this.#canvas.height);
    gl.clear(gl.COLOR_BUFFER_BIT);
  }

  draw(shapes: readonly FxShape[], count: number): void {
    const gl = this.#gl;
    if (gl.isContextLost()) return;
    this.#ready();
    this.clear();
    gl.useProgram(this.#program);
    const at = this.#at;
    gl.uniform2f(at.u_surface, this.#canvas.width, this.#canvas.height);
    gl.uniform1f(at.u_edge, FX_EDGE);
    for (let index = 0; index < count; index++) {
      const shape = shapes[index];
      if (!shape) continue;
      gl.uniform4f(at.u_bounds, shape.left, shape.top, shape.right, shape.bottom);
      gl.uniform1i(at.u_kind, shape.kind);
      gl.uniform2fv(at.u_points, shape.points);
      gl.uniform1i(at.u_count, shape.count);
      gl.uniform4f(at.u_box, shape.centreX, shape.centreY, shape.halfWidth, shape.halfHeight);
      gl.uniform1f(at.u_radius, shape.radius);
      gl.uniform1f(at.u_ring, shape.ring);
      const [r, g, b] = channels(shape.color);
      gl.uniform3f(at.u_color, r, g, b);
      gl.uniform1f(at.u_alpha, shape.alpha);
      gl.uniform4f(at.u_fade, shape.fadeFromX, shape.fadeFromY, shape.fadeToX, shape.fadeToY);
      gl.uniform1f(at.u_fadePower, shape.fadePower);
      gl.uniform2f(at.u_halo, shape.haloRadius, shape.haloStrength);
      gl.uniform4f(at.u_hole, shape.holeLeft, shape.holeTop, shape.holeRight, shape.holeBottom);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }
  }

  onLost(listener: () => void): void {
    this.#lost = listener;
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#canvas.removeEventListener("webglcontextlost", this.#onContextLost);
    this.#lost = undefined;
    const gl = this.#gl;
    for (const shader of this.#shaders) gl.deleteShader(shader);
    gl.deleteProgram(this.#program);
    // A page may keep only a few contexts alive: this one is given up at once.
    gl.getExtension("WEBGL_lose_context")?.loseContext();
  }

  /**
   * Checks the program once, before its first use: here the browser waits for a
   * compile it did in the background, and a program that did not link is reported.
   */
  #ready(): void {
    if (this.#checked) return;
    this.#checked = true;
    const gl = this.#gl;
    if (!gl.getProgramParameter(this.#program, gl.LINK_STATUS) && !gl.isContextLost()) {
      const log = this.#shaders.map((shader) => gl.getShaderInfoLog(shader) ?? "").join("\n");
      throw new Error(
        `the cursor effects' program failed to link: ${gl.getProgramInfoLog(this.#program) ?? ""}\n${log}`,
      );
    }
    for (const name of FX_UNIFORMS) this.#at[name] = gl.getUniformLocation(this.#program, name);
  }
}
