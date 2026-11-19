/**
 * The renderer's entry point for a window.
 *
 * A window has one `RenderHost`. The one slow step is the browser's start of its GPU
 * process, which asking for a WebGPU device waits for without blocking, and making a
 * WebGL2 context waits for while blocking the page. `create` waits for it; `start` does
 * not: its renderers begin with Canvas 2D and move to the GPU tiers once the GPU has
 * answered. Renderers for panes are made at once, and
 * a pane that loses its tier can start the next one within the same frame.
 *
 * The host also owns the window's frame loop, and keeps count of what its renderers
 * hold: a pane that would need one WebGL2 context more than a page may keep alive draws
 * with the next tier instead.
 */

import { canvasFonts } from "./atlas/raster.ts";
import { Canvas2dBackend } from "./canvas2d/backend.ts";
import { TIER_ORDER } from "./constants.ts";
import { Canvas2dFx } from "./fx/canvas2d.ts";
import { CursorEffects, type FxSurface } from "./fx/effects.ts";
import type { FxBackend } from "./fx/shape.ts";
import { FxUnavailableError, WebGl2Fx } from "./fx/webgl2.ts";
import { FrameLoop } from "./loop.ts";
import { RenderPool, type RenderStats } from "./pool.ts";
import { TierUnavailableError, type RendererOptions, type Surface } from "./select.ts";
import type { CellMetrics, CellRenderer, Size, TerminalTheme, Tier } from "./types.ts";
import { WebGl2Backend } from "./webgl2/backend.ts";
import { WebGpuBackend } from "./webgpu/backend.ts";
import { acquireGpu, gpuOffered, type GpuShared } from "./webgpu/device.ts";

/** Choices made when a host is created. */
export interface RenderHostOptions {
  /** The tiers to try and their order. Defaults to `TIER_ORDER`. */
  tiers?: readonly Tier[];
}

/** A place in the page for a renderer's canvas. */
function domSurface(container: HTMLElement): Surface<HTMLCanvasElement> {
  return {
    createCanvas() {
      const canvas = document.createElement("canvas");
      // Styles are set through the style object: a policy that forbids inline style
      // attributes still allows it.
      canvas.style.display = "block";
      return canvas;
    },
    show(canvas) {
      container.replaceChildren(canvas);
    },
    remove(canvas) {
      canvas.remove();
    },
    setCssSize(canvas, size: Size) {
      canvas.style.width = `${size.width}px`;
      canvas.style.height = `${size.height}px`;
    },
  };
}

/** The place in the page for the cursor's effects: a canvas over the whole window. */
function fxSurface(container: HTMLElement): FxSurface<HTMLCanvasElement> {
  return {
    createCanvas() {
      const canvas = document.createElement("canvas");
      canvas.style.display = "block";
      canvas.style.pointerEvents = "none";
      return canvas;
    },
    show(canvas) {
      container.replaceChildren(canvas);
    },
    remove(canvas) {
      canvas.remove();
    },
    setCssSize(canvas, size: Size) {
      canvas.style.width = `${size.width}px`;
      canvas.style.height = `${size.height}px`;
    },
    setVisible(canvas, visible) {
      // Hidden, not taken out of the layout: showing it again lays nothing out.
      canvas.style.visibility = visible ? "visible" : "hidden";
    },
  };
}

/** Makes renderers for the panes of one window. */
export class RenderHost {
  /** The window's frame loop. */
  readonly loop: FrameLoop;
  readonly #order: readonly Tier[];
  readonly #pool: RenderPool<HTMLCanvasElement>;
  #gpu: GpuShared | undefined;
  /**
   * Whether the GPU tiers may be tried. A host made by `start` waits for the GPU to
   * answer first, since until then a WebGL2 context would hold up the page.
   */
  #gpuAnswered = true;
  /** Cursor effects made before the GPU answered, to be told when it has. */
  readonly #effects = new Set<CursorEffects<HTMLCanvasElement>>();
  #disposed = false;

  private constructor(order: readonly Tier[], gpu: GpuShared | undefined) {
    this.#order = order;
    this.#gpu = gpu;
    this.#pool = new RenderPool<HTMLCanvasElement>(
      order,
      {
        webgpu: (canvas) => {
          if (!this.#gpu) throw new TierUnavailableError("webgpu", "no device");
          return new WebGpuBackend(this.#gpu, canvas);
        },
        webgl2: (canvas) => {
          if (!this.#gpuAnswered) throw new TierUnavailableError("webgl2", "the GPU is starting");
          return new WebGl2Backend(canvas);
        },
        canvas2d: (canvas) => new Canvas2dBackend(canvas),
      },
      canvasFonts,
    );
    this.loop = new FrameLoop();
    // Once the device is gone no new pane can use it either.
    gpu?.onLost(() => (this.#gpu = undefined));
  }

  /** Prepare a window for rendering, once the WebGPU device has been asked for. */
  static async create(options: RenderHostOptions = {}): Promise<RenderHost> {
    const order = options.tiers ?? TIER_ORDER;
    const gpu = order.includes("webgpu") ? await acquireGpu() : undefined;
    return new RenderHost(order, gpu);
  }

  /**
   * Prepare a window for rendering at once. Renderers made before the GPU has answered
   * the request for a WebGPU device draw with Canvas 2D, and move to the first GPU tier
   * that works at their next frame after it has; `gpuSettled` resolves then. Where there
   * is no WebGPU to ask, the GPU tiers are tried at once, as `create` tries them.
   */
  static start(options: RenderHostOptions = {}): RenderHost {
    const order = options.tiers ?? TIER_ORDER;
    const host = new RenderHost(order, undefined);
    if (order.includes("webgpu") && order.includes("canvas2d") && gpuOffered()) {
      host.#gpuAnswered = false;
      host.gpuSettled = acquireGpu().then((gpu) => host.#answered(gpu));
    }
    return host;
  }

  /** Resolves once the GPU has answered the request for a device, granted or not. */
  gpuSettled: Promise<void> = Promise.resolve();

  #answered(gpu: GpuShared | undefined): void {
    if (this.#disposed) {
      gpu?.dispose();
      return;
    }
    this.#gpuAnswered = true;
    if (gpu) {
      this.#gpu = gpu;
      gpu.onLost(() => (this.#gpu = undefined));
    }
    this.#pool.upgrade();
    for (const effects of this.#effects) effects.gpuAnswered();
    this.#effects.clear();
  }

  /**
   * The tiers a new renderer will try, in order. WebGPU is listed only while this
   * window has a device; the others are found out by trying.
   */
  get tiers(): readonly Tier[] {
    return this.#order.filter((tier) => tier !== "webgpu" || this.#gpu !== undefined);
  }

  /**
   * Make a renderer that draws into `container`.
   *
   * The renderer puts a canvas of its own inside the container and replaces it if it
   * has to change tier, so the container should hold nothing else. The caller keeps the
   * renderer's size up to date with `resize` and draws frames with `draw`.
   *
   * A window may keep only so many WebGL2 contexts alive. A renderer that would be one
   * too many starts on the tier after WebGL2; one told to use WebGL2 and nothing else
   * cannot be made then, and this throws.
   */
  createRenderer(
    container: HTMLElement,
    theme: TerminalTheme,
    options: RendererOptions = {},
  ): CellRenderer {
    if (this.#disposed) throw new Error("this render host has been disposed");
    return this.#pool.createRenderer(domSurface(container), theme, options);
  }

  /**
   * The cell geometry a renderer made here with `theme` reports at the device pixel
   * ratio `dpr`. Nothing is put in the page and no renderer is made, so the grid a pane
   * will have can be worked out before the pane exists.
   */
  measureCells(theme: TerminalTheme, dpr: number): CellMetrics {
    return this.#pool.measureCells(theme, dpr);
  }

  /**
   * Make the window's cursor effects, drawn on a canvas of their own inside
   * `container`, which should cover the window and hold nothing else. They are drawn by
   * the window's frame loop after the panes. While they draw with WebGL2 their context
   * counts against the window's limit, so panes and effects together stay within it;
   * with no room left, or where this window may not use WebGL2, they use Canvas 2D.
   */
  createCursorEffects(container: HTMLElement): CursorEffects<HTMLCanvasElement> {
    if (this.#disposed) throw new Error("this render host has been disposed");
    const webgl2 = this.#order.includes("webgl2")
      ? (canvas: HTMLCanvasElement): FxBackend => {
          if (!this.#gpuAnswered) throw new FxUnavailableError("the GPU is starting");
          const release = this.#pool.reserve("webgl2");
          if (!release) throw new FxUnavailableError("the window has no WebGL2 context to spare");
          try {
            const backend = new WebGl2Fx(canvas);
            const dispose = backend.dispose.bind(backend);
            backend.dispose = () => {
              dispose();
              release();
            };
            return backend;
          } catch (error) {
            release();
            throw error;
          }
        }
      : undefined;
    const effects: CursorEffects<HTMLCanvasElement> = new CursorEffects(
      fxSurface(container),
      { webgl2, canvas2d: (canvas) => new Canvas2dFx(canvas) },
      () => this.loop.requestFrame(),
      (): void => this.loop.remove(effects),
    );
    this.loop.add(effects, { after: true });
    if (!this.#gpuAnswered) this.#effects.add(effects);
    return effects;
  }

  /** What the renderers made here hold now: how many there are, and their glyph pages. */
  stats(): RenderStats {
    return this.#pool.stats();
  }

  /** Stop the frame loop and give up the device. Renderers made here stop working. */
  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.loop.dispose();
    this.#gpu?.dispose();
    this.#gpu = undefined;
  }
}
