/**
 * The renderer's entry point for a window.
 *
 * A window has one `RenderHost`. Creating it does the one slow, asynchronous step, which
 * is asking for a WebGPU device. After that, renderers for panes are made at once, and a
 * pane that loses its tier can start the next one within the same frame.
 *
 * The host also owns the window's frame loop, and keeps count of what its renderers
 * hold: a pane that would need one WebGL2 context more than a page may keep alive draws
 * with the next tier instead.
 */

import { canvasFonts } from "./atlas/raster.ts";
import { Canvas2dBackend } from "./canvas2d/backend.ts";
import { TIER_ORDER } from "./constants.ts";
import { FrameLoop } from "./loop.ts";
import { RenderPool, type RenderStats } from "./pool.ts";
import { TierUnavailableError, type RendererOptions, type Surface } from "./select.ts";
import type { CellMetrics, CellRenderer, Size, TerminalTheme, Tier } from "./types.ts";
import { WebGl2Backend } from "./webgl2/backend.ts";
import { WebGpuBackend } from "./webgpu/backend.ts";
import { acquireGpu, type GpuShared } from "./webgpu/device.ts";

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

/** Makes renderers for the panes of one window. */
export class RenderHost {
  /** The window's frame loop. */
  readonly loop: FrameLoop;
  readonly #order: readonly Tier[];
  readonly #pool: RenderPool<HTMLCanvasElement>;
  #gpu: GpuShared | undefined;
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
        webgl2: (canvas) => new WebGl2Backend(canvas),
        canvas2d: (canvas) => new Canvas2dBackend(canvas),
      },
      canvasFonts,
    );
    this.loop = new FrameLoop();
    // Once the device is gone no new pane can use it either.
    gpu?.onLost(() => (this.#gpu = undefined));
  }

  /** Prepare a window for rendering. */
  static async create(options: RenderHostOptions = {}): Promise<RenderHost> {
    const order = options.tiers ?? TIER_ORDER;
    const gpu = order.includes("webgpu") ? await acquireGpu() : undefined;
    return new RenderHost(order, gpu);
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
