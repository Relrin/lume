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
import {
  CUSTOM_UNIFORMS,
  CustomEffectError,
  customFragment,
  type CustomDraw,
  type CustomUniform,
} from "./custom.ts";
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

/** A plugin's effect, made into a program. */
interface Custom {
  readonly program: WebGLProgram;
  readonly shaders: readonly WebGLShader[];
  readonly at: Record<CustomUniform, WebGLUniformLocation | null>;
  /** Whether it was looked at once the browser finished building it. */
  checked: boolean;
  /** Why it cannot be drawn with, once that is known. */
  failed?: string;
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
  readonly #custom = new Map<string, Custom>();
  /**
   * How to ask whether a program is built without waiting for it, where the browser
   * builds programs beside the page; null where it does not say.
   */
  readonly #parallel: { readonly COMPLETION_STATUS_KHR: number } | null;
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
    // Asked for so that the programs are compiled off the main thread, and so that
    // whether one is done can be asked without waiting for it.
    this.#parallel = gl.getExtension("KHR_parallel_shader_compile") as {
      readonly COMPLETION_STATUS_KHR: number;
    } | null;

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
    this.clear();
    // A program still being built draws nothing this frame, which does not wait.
    if (!this.#ready()) return;
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

  prepareCustom(id: string, source: string): void {
    if (this.#custom.has(id)) return;
    const gl = this.#gl;
    const program = gl.createProgram();
    if (!program) throw new CustomEffectError("could not create a program");
    const shaders = [
      compile(gl, gl.VERTEX_SHADER, FX_VERTEX),
      compile(gl, gl.FRAGMENT_SHADER, customFragment(source)),
    ];
    for (const shader of shaders) gl.attachShader(program, shader);
    gl.linkProgram(program);
    const custom: Custom = {
      program,
      shaders,
      at: {} as Record<CustomUniform, WebGLUniformLocation | null>,
      checked: false,
    };
    this.#custom.set(id, custom);
    // Where the browser cannot be asked whether it is done, the waiting is done here,
    // when the effect is chosen, and not when the cursor first jumps.
    if (!this.#parallel) this.#check(custom);
  }

  customReady(id: string): boolean {
    const gl = this.#gl;
    const custom = this.#custom.get(id);
    if (!custom || gl.isContextLost()) return false;
    if (!custom.checked) {
      const done =
        !this.#parallel ||
        (gl.getProgramParameter(custom.program, this.#parallel.COMPLETION_STATUS_KHR) as boolean);
      if (!done) return false;
      this.#check(custom);
    }
    if (custom.failed !== undefined) throw new CustomEffectError(custom.failed);
    return true;
  }

  /** Looks at a program the browser has finished building, or waits for it to. */
  #check(custom: Custom): void {
    const gl = this.#gl;
    custom.checked = true;
    if (!gl.getProgramParameter(custom.program, gl.LINK_STATUS)) {
      const log = custom.shaders
        .map((shader) => gl.getShaderInfoLog(shader) ?? "")
        .join("\n")
        .trim();
      custom.failed = log === "" ? (gl.getProgramInfoLog(custom.program) ?? "") : log;
    } else {
      for (const name of CUSTOM_UNIFORMS) {
        custom.at[name] = gl.getUniformLocation(custom.program, name);
      }
    }
  }

  drawCustom(id: string, frame: CustomDraw): void {
    const gl = this.#gl;
    const custom = this.#custom.get(id);
    // One not looked at yet may still be building: it is not waited for here.
    if (!custom || !custom.checked || gl.isContextLost()) return;
    if (custom.failed !== undefined) throw new CustomEffectError(custom.failed);
    const at = custom.at;
    gl.useProgram(custom.program);
    gl.uniform2f(at.u_surface, this.#canvas.width, this.#canvas.height);
    gl.uniform4f(at.u_bounds, ...frame.bounds);
    gl.uniform2f(at.lume_resolution, this.#canvas.width, this.#canvas.height);
    gl.uniform1f(at.lume_progress, frame.progress);
    gl.uniform1f(at.lume_time, frame.time);
    gl.uniform4f(at.lume_from, ...frame.from);
    gl.uniform4f(at.lume_to, ...frame.to);
    const [r, g, b] = channels(frame.color);
    gl.uniform3f(at.lume_color, r, g, b);
    gl.uniform4f(at.lume_hole, ...frame.hole);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }

  forgetCustom(id: string): void {
    const custom = this.#custom.get(id);
    if (!custom) return;
    this.#custom.delete(id);
    const gl = this.#gl;
    for (const shader of custom.shaders) gl.deleteShader(shader);
    gl.deleteProgram(custom.program);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#canvas.removeEventListener("webglcontextlost", this.#onContextLost);
    this.#lost = undefined;
    const gl = this.#gl;
    for (const id of [...this.#custom.keys()]) this.forgetCustom(id);
    for (const shader of this.#shaders) gl.deleteShader(shader);
    gl.deleteProgram(this.#program);
    // A page may keep only a few contexts alive: this one is given up at once.
    gl.getExtension("WEBGL_lose_context")?.loseContext();
  }

  /**
   * Whether the program can be drawn with. It is looked at once, when the browser
   * says it has built it, and a program that did not link is reported then. Where the
   * browser cannot be asked, the first use waits for it.
   */
  #ready(): boolean {
    if (this.#checked) return true;
    const gl = this.#gl;
    if (
      this.#parallel &&
      !(gl.getProgramParameter(this.#program, this.#parallel.COMPLETION_STATUS_KHR) as boolean)
    ) {
      return false;
    }
    this.#checked = true;
    if (!gl.getProgramParameter(this.#program, gl.LINK_STATUS) && !gl.isContextLost()) {
      const log = this.#shaders.map((shader) => gl.getShaderInfoLog(shader) ?? "").join("\n");
      throw new Error(
        `the cursor effects' program failed to link: ${gl.getProgramInfoLog(this.#program) ?? ""}\n${log}`,
      );
    }
    for (const name of FX_UNIFORMS) this.#at[name] = gl.getUniformLocation(this.#program, name);
    return true;
  }
}
