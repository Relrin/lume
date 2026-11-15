/**
 * The renderer's entry point for a window.
 *
 * A window has one `RenderHost`. Creating it does the one slow, asynchronous step, which
 * is asking for a WebGPU device. After that, renderers for panes are made at once, and a
 * pane that loses its tier can start the next one within the same frame.
 *
 * The host also owns the window's frame loop.
 */

import { canvasFonts } from "./atlas/raster.ts";
import { Canvas2dBackend } from "./canvas2d/backend.ts";
import { TIER_ORDER } from "./constants.ts";
import { FrameLoop } from "./loop.ts";
import {
  ManagedRenderer,
  TierUnavailableError,
  type RendererOptions,
  type Surface,
  type TierFactory,
} from "./select.ts";
import type { CellRenderer, Size, TerminalTheme, Tier, TierBackend } from "./types.ts";
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
  #gpu: GpuShared | undefined;
  #disposed = false;

  private constructor(order: readonly Tier[], gpu: GpuShared | undefined) {
    this.#order = order;
    this.#gpu = gpu;
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
   */
  createRenderer(
    container: HTMLElement,
    theme: TerminalTheme,
    options: RendererOptions = {},
  ): CellRenderer {
    if (this.#disposed) throw new Error("this render host has been disposed");
    const make: Record<Tier, (canvas: HTMLCanvasElement) => TierBackend> = {
      webgpu: (canvas) => {
        if (!this.#gpu) throw new TierUnavailableError("webgpu", "no device");
        return new WebGpuBackend(this.#gpu, canvas);
      },
      webgl2: (canvas) => new WebGl2Backend(canvas),
      canvas2d: (canvas) => new Canvas2dBackend(canvas),
    };
    const factories: TierFactory<HTMLCanvasElement>[] = this.#order.map((tier) => ({
      tier,
      create: make[tier],
    }));
    return new ManagedRenderer(domSurface(container), factories, canvasFonts, theme, options);
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
