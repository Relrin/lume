/**
 * The WebGPU tier.
 *
 * Each plane of instance records is one vertex buffer drawn with one instanced call.
 * Atlas pages are layers of two array textures on the window's shared device. Every
 * frame redraws everything from the buffers; only the bytes of changed rows are sent to
 * the GPU.
 */

import type { GlyphAtlas } from "../atlas/atlas.ts";
import { GLYPH, PLANE_ORDER, type PlaneName, type Planes } from "../layout.ts";
import { TierUnavailableError } from "../select.ts";
import type { AtlasSink, PresentInfo, Tier, TierBackend } from "../types.ts";
import type { GpuPages, GpuShared } from "./device.ts";
import { BUFFER_COPY_DST, BUFFER_UNIFORM, BUFFER_VERTEX } from "./flags.ts";

/** Bytes of the uniform block: the surface size as two floats, padded to sixteen. */
const UNIFORM_BYTES = 16;

/** Draws instance records with WebGPU. */
export class WebGpuBackend implements TierBackend {
  readonly tier: Tier = "webgpu";
  readonly #shared: GpuShared;
  readonly #canvas: HTMLCanvasElement;
  readonly #context: GPUCanvasContext;
  readonly #uniforms: GPUBuffer;
  readonly #uniformData = new Float32Array(UNIFORM_BYTES / 4);
  /** The key of the shared atlas this pane holds, and that atlas and its pages. */
  #atlasKey: string | undefined;
  #atlas: GlyphAtlas | undefined;
  #pages: GpuPages | undefined;
  #bindGroup: GPUBindGroup | undefined;
  /** The pages and the version of them the bind group was made for. */
  #boundPages: GpuPages | undefined;
  #boundVersion = -1;
  #planes: Planes | undefined;
  #buffers: Partial<Record<PlaneName, GPUBuffer>> = {};
  #stopListening: (() => void) | undefined;
  #disposed = false;

  constructor(shared: GpuShared, canvas: HTMLCanvasElement) {
    if (shared.lost !== undefined) throw new TierUnavailableError("webgpu", shared.lost);
    const context = canvas.getContext("webgpu");
    if (!(context instanceof GPUCanvasContext)) {
      throw new TierUnavailableError("webgpu", "no WebGPU canvas context");
    }
    this.#shared = shared;
    this.#canvas = canvas;
    this.#context = context;
    // The terminal is opaque, which lets the compositor skip blending the canvas.
    context.configure({ device: shared.device, format: shared.format, alphaMode: "opaque" });
    this.#uniforms = shared.device.createBuffer({
      size: UNIFORM_BYTES,
      usage: BUFFER_UNIFORM | BUFFER_COPY_DST,
    });
  }

  atlas(key: string, create: (sink: AtlasSink) => GlyphAtlas): GlyphAtlas {
    if (this.#atlas && this.#atlasKey === key) return this.#atlas;
    const { atlas, pages } = this.#shared.acquireAtlas(key, create);
    // Glyphs of the font shown before are of no further use to this pane.
    this.#releaseAtlas();
    this.#atlasKey = key;
    this.#atlas = atlas;
    this.#pages = pages;
    return atlas;
  }

  configure(width: number, height: number): void {
    this.#canvas.width = width;
    this.#canvas.height = height;
  }

  setPlanes(planes: Planes): void {
    this.#destroyBuffers();
    this.#planes = planes;
    for (const name of PLANE_ORDER) {
      this.#buffers[name] = this.#shared.device.createBuffer({
        size: planes[name].bytes.byteLength,
        usage: BUFFER_VERTEX | BUFFER_COPY_DST,
      });
    }
  }

  upload(name: PlaneName, byteStart: number, byteEnd: number): void {
    const buffer = this.#buffers[name];
    const plane = this.#planes?.[name];
    if (!buffer || !plane || byteEnd <= byteStart) return;
    // Records are multiples of four bytes, as the offsets and sizes here must be.
    this.#shared.device.queue.writeBuffer(
      buffer,
      byteStart,
      plane.bytes,
      byteStart,
      byteEnd - byteStart,
    );
  }

  present(info: PresentInfo): void {
    const { device } = this.#shared;
    if (this.#disposed || this.#shared.lost !== undefined || !this.#pages) return;

    if (this.#uniformData[0] !== info.width || this.#uniformData[1] !== info.height) {
      this.#uniformData[0] = info.width;
      this.#uniformData[1] = info.height;
      device.queue.writeBuffer(this.#uniforms, 0, this.#uniformData);
    }
    // Growing an atlas replaces its textures, and a bind group names the old ones.
    if (this.#boundPages !== this.#pages || this.#boundVersion !== this.#pages.version) {
      this.#bindGroup = device.createBindGroup({
        layout: this.#shared.bindGroupLayout,
        entries: [
          { binding: 0, resource: { buffer: this.#uniforms } },
          { binding: 1, resource: this.#pages.view("mask") },
          { binding: 2, resource: this.#pages.view("color") },
        ],
      });
      this.#boundPages = this.#pages;
      // Read after the views are taken: taking one can itself make a first texture.
      this.#boundVersion = this.#pages.version;
    }
    if (!this.#bindGroup) return;

    const clear = info.clear;
    const encoder = device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [
        {
          view: this.#context.getCurrentTexture().createView(),
          clearValue: {
            r: (clear & 0xff) / 255,
            g: ((clear >>> 8) & 0xff) / 255,
            b: ((clear >>> 16) & 0xff) / 255,
            a: 1,
          },
          loadOp: "clear",
          storeOp: "store",
        },
      ],
    });
    pass.setBindGroup(0, this.#bindGroup);
    for (const name of PLANE_ORDER) {
      const buffer = this.#buffers[name];
      const plane = this.#planes?.[name];
      if (!buffer || !plane) continue;
      const count =
        name === "overlayRect"
          ? info.overlayRects
          : name === "overlayGlyph"
            ? info.overlayGlyphs
            : plane.capacity;
      if (count === 0) continue;
      pass.setPipeline(
        plane.layout === GLYPH ? this.#shared.glyphPipeline : this.#shared.rectPipeline,
      );
      pass.setVertexBuffer(0, buffer);
      pass.draw(4, count);
    }
    pass.end();
    device.queue.submit([encoder.finish()]);
  }

  onLost(listener: (reason: string) => void): void {
    this.#stopListening?.();
    this.#stopListening = this.#shared.onLost(listener);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#stopListening?.();
    this.#destroyBuffers();
    this.#uniforms.destroy();
    // The device belongs to the window and stays. The atlas goes with its last pane.
    this.#releaseAtlas();
    this.#context.unconfigure();
  }

  #releaseAtlas(): void {
    if (this.#atlasKey !== undefined) this.#shared.releaseAtlas(this.#atlasKey);
    this.#atlasKey = undefined;
    this.#atlas = undefined;
    this.#pages = undefined;
  }

  #destroyBuffers(): void {
    for (const buffer of Object.values(this.#buffers)) buffer.destroy();
    this.#buffers = {};
  }
}
