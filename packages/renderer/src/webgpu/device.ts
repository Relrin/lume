/**
 * The WebGPU device of a window and what its panes share.
 *
 * Getting a device is asynchronous and takes a noticeable moment, so it happens once,
 * when the window starts. Every pane then draws on the same device, which is also what
 * lets them share glyph textures: an atlas made for one pane serves all panes that use
 * the same font at the same size.
 */

import type { GlyphAtlas } from "../atlas/atlas.ts";
import { SharedAtlases, type SharedAtlas } from "../atlas/shared.ts";
import { ATLAS_PAGE_SIZE, WEBGPU_POWER_PREFERENCE } from "../constants.ts";
import { GLYPH, RECT } from "../layout.ts";
import type { AtlasSink, PageKind } from "../types.ts";
import {
  STAGE_FRAGMENT,
  STAGE_VERTEX,
  TEXTURE_BINDING,
  TEXTURE_COPY_DST,
  TEXTURE_COPY_SRC,
} from "./flags.ts";
import { vertexBufferPlan } from "./plan.ts";
import { SHADER } from "./shaders.ts";

const FORMATS: Readonly<Record<PageKind, GPUTextureFormat>> = {
  mask: "r8unorm",
  color: "rgba8unorm",
};

/** Atlas pages as layers of array textures on a device. */
export class GpuPages implements AtlasSink {
  readonly #device: GPUDevice;
  readonly #textures: Record<PageKind, GPUTexture | undefined> = {
    mask: undefined,
    color: undefined,
  };
  readonly #layers: Record<PageKind, number> = { mask: 0, color: 0 };
  /** Changes whenever a texture is replaced, so users know to bind the new one. */
  version = 0;

  constructor(device: GPUDevice) {
    this.#device = device;
  }

  ensurePages(kind: PageKind, count: number): void {
    if (count <= this.#layers[kind]) return;
    // An array texture cannot gain layers, so a larger one is made and the pages
    // already drawn are copied across.
    const texture = this.#create(kind, count, ATLAS_PAGE_SIZE);
    const old = this.#textures[kind];
    if (old && this.#layers[kind] > 0) {
      const encoder = this.#device.createCommandEncoder();
      encoder.copyTextureToTexture({ texture: old }, { texture }, [
        ATLAS_PAGE_SIZE,
        ATLAS_PAGE_SIZE,
        this.#layers[kind],
      ]);
      this.#device.queue.submit([encoder.finish()]);
    }
    old?.destroy();
    this.#textures[kind] = texture;
    this.#layers[kind] = count;
    this.version++;
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
    const texture = this.#textures[kind];
    if (!texture) return;
    this.#device.queue.writeTexture(
      { texture, origin: [x, y, page] },
      pixels as Uint8Array<ArrayBuffer>,
      { bytesPerRow: width * (kind === "mask" ? 1 : 4), rowsPerImage: height },
      [width, height, 1],
    );
  }

  /** A view of the pages of a kind, to bind for drawing. */
  view(kind: PageKind): GPUTextureView {
    // A kind with no pages yet still needs something bound: one empty texel.
    if (!this.#textures[kind]) {
      this.#textures[kind] = this.#create(kind, 1, 1);
      this.version++;
    }
    // Named as an array even with one layer, which would otherwise be a plain texture
    // and not match the shader.
    return this.#textures[kind].createView({ dimension: "2d-array" });
  }

  dispose(): void {
    this.#textures.mask?.destroy();
    this.#textures.color?.destroy();
    this.#textures.mask = undefined;
    this.#textures.color = undefined;
    this.#layers.mask = 0;
    this.#layers.color = 0;
    this.version++;
  }

  #create(kind: PageKind, layers: number, size: number): GPUTexture {
    return this.#device.createTexture({
      size: [size, size, layers],
      format: FORMATS[kind],
      usage: TEXTURE_BINDING | TEXTURE_COPY_DST | TEXTURE_COPY_SRC,
    });
  }
}

/** A device and everything on it that panes share. */
export class GpuShared {
  readonly device: GPUDevice;
  /** The surface format the browser prefers, which avoids a conversion on present. */
  readonly format: GPUTextureFormat;
  readonly bindGroupLayout: GPUBindGroupLayout;
  readonly rectPipeline: GPURenderPipeline;
  readonly glyphPipeline: GPURenderPipeline;
  readonly #atlases = new SharedAtlases<GpuPages>();
  readonly #lostListeners = new Set<(reason: string) => void>();
  /** Set once the device is gone, by loss or by `dispose`. */
  lost: string | undefined;

  constructor(device: GPUDevice, format: GPUTextureFormat) {
    this.device = device;
    this.format = format;

    this.bindGroupLayout = device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: STAGE_VERTEX, buffer: { type: "uniform" } },
        {
          binding: 1,
          visibility: STAGE_FRAGMENT,
          texture: { sampleType: "float", viewDimension: "2d-array" },
        },
        {
          binding: 2,
          visibility: STAGE_FRAGMENT,
          texture: { sampleType: "float", viewDimension: "2d-array" },
        },
      ],
    });
    const module = device.createShaderModule({ code: SHADER });
    const layout = device.createPipelineLayout({ bindGroupLayouts: [this.bindGroupLayout] });
    // Colours arrive premultiplied.
    const blend: GPUBlendComponent = { srcFactor: "one", dstFactor: "one-minus-src-alpha" };
    const pipeline = (kind: "rect" | "glyph") =>
      device.createRenderPipeline({
        layout,
        vertex: {
          module,
          entryPoint: `${kind}_vertex`,
          buffers: [vertexBufferPlan(kind === "rect" ? RECT : GLYPH)],
        },
        fragment: {
          module,
          entryPoint: `${kind}_fragment`,
          targets: [{ format, blend: { color: blend, alpha: blend } }],
        },
        primitive: { topology: "triangle-strip" },
      });
    this.rectPipeline = pipeline("rect");
    this.glyphPipeline = pipeline("glyph");

    void device.lost.then((info) => {
      // A device this object destroyed itself is not a loss to report.
      if (this.lost !== undefined) return;
      this.lost = info.message || "the WebGPU device was lost";
      for (const listener of [...this.#lostListeners]) listener(this.lost);
    });
  }

  /**
   * Takes the atlas for a font key and the pages it draws into, shared by every pane
   * that asks for the key. Each call must be matched by one `releaseAtlas`.
   */
  acquireAtlas(key: string, create: (sink: AtlasSink) => GlyphAtlas): SharedAtlas<GpuPages> {
    return this.#atlases.acquire(key, () => new GpuPages(this.device), create);
  }

  /** Lets go of the atlas for a font key. Its textures are freed with its last user. */
  releaseAtlas(key: string): void {
    this.#atlases.release(key);
  }

  /** How many atlases are alive on the device. */
  get atlasCount(): number {
    return this.#atlases.size;
  }

  /** Hear about the loss of the device. Returns how to stop listening. */
  onLost(listener: (reason: string) => void): () => void {
    this.#lostListeners.add(listener);
    return () => this.#lostListeners.delete(listener);
  }

  dispose(): void {
    if (this.lost === undefined) this.lost = "disposed";
    this.#lostListeners.clear();
    this.#atlases.clear();
    this.device.destroy();
  }
}

/**
 * Get a device, or `undefined` where WebGPU is missing, is turned off, or would be
 * emulated in software.
 */
export async function acquireGpu(): Promise<GpuShared | undefined> {
  // The type says `navigator.gpu` always exists. It does not.
  if (typeof navigator === "undefined" || !("gpu" in navigator) || !navigator.gpu) {
    return undefined;
  }
  try {
    const adapter = await navigator.gpu.requestAdapter({
      powerPreference: WEBGPU_POWER_PREFERENCE,
    });
    // A software adapter draws slower than the WebGL2 or Canvas 2D tiers would.
    if (!adapter || adapter.info.isFallbackAdapter) return undefined;
    const device = await adapter.requestDevice();
    return new GpuShared(device, navigator.gpu.getPreferredCanvasFormat());
  } catch {
    return undefined;
  }
}
