import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { GlyphAtlas } from "../src/atlas/atlas.ts";
import { SharedAtlases } from "../src/atlas/shared.ts";
import { cellMetrics } from "../src/metrics.ts";
import { FakeRasteriser, RecordingSink } from "../src/testing/index.ts";
import type { AtlasSink } from "../src/types.ts";
import { WebGpuBackend } from "../src/webgpu/backend.ts";
import { GpuShared } from "../src/webgpu/device.ts";

const METRICS = cellMetrics(
  { advance: 8, ascent: 10.4, descent: 2.9 },
  { family: "Test Mono", sizePx: 14 },
  0,
  1,
);

function makeAtlas(sink: AtlasSink): GlyphAtlas {
  return new GlyphAtlas(sink, new FakeRasteriser(METRICS), METRICS);
}

class Pages extends RecordingSink {
  disposed = 0;

  dispose(): void {
    this.disposed++;
  }
}

describe("atlases panes share", () => {
  it("hands every user of a key the same atlas and makes its pages once", () => {
    const shared = new SharedAtlases<Pages>();
    let made = 0;
    const pages = () => {
      made++;
      return new Pages();
    };
    const first = shared.acquire("a", pages, makeAtlas);
    const second = shared.acquire("a", pages, makeAtlas);
    expect(second.atlas).toBe(first.atlas);
    expect(second.pages).toBe(first.pages);
    expect(made).toBe(1);
    expect(shared.users("a")).toBe(2);
    expect(shared.size).toBe(1);
  });

  it("frees the pages with the last user and not before", () => {
    const shared = new SharedAtlases<Pages>();
    const { pages } = shared.acquire("a", () => new Pages(), makeAtlas);
    shared.acquire("a", () => new Pages(), makeAtlas);

    shared.release("a");
    expect(pages.disposed).toBe(0);
    expect(shared.size).toBe(1);

    shared.release("a");
    expect(pages.disposed).toBe(1);
    expect(shared.size).toBe(0);
    expect(shared.users("a")).toBe(0);
  });

  it("makes a new atlas for a key whose atlas was freed", () => {
    const shared = new SharedAtlases<Pages>();
    const before = shared.acquire("a", () => new Pages(), makeAtlas);
    shared.release("a");
    const after = shared.acquire("a", () => new Pages(), makeAtlas);
    expect(after.atlas).not.toBe(before.atlas);
    expect(after.pages.disposed).toBe(0);
  });

  it("keeps the atlases of different keys apart", () => {
    const shared = new SharedAtlases<Pages>();
    const a = shared.acquire("a", () => new Pages(), makeAtlas);
    const b = shared.acquire("b", () => new Pages(), makeAtlas);
    expect(b.atlas).not.toBe(a.atlas);

    shared.release("a");
    expect(a.pages.disposed).toBe(1);
    expect(b.pages.disposed).toBe(0);
    expect(shared.size).toBe(1);
  });

  it("ignores a release of a key that has no atlas", () => {
    const shared = new SharedAtlases<Pages>();
    shared.release("nothing");
    const { pages } = shared.acquire("a", () => new Pages(), makeAtlas);
    shared.release("a");
    shared.release("a");
    expect(pages.disposed).toBe(1);
  });

  it("frees everything at once, whoever still uses it", () => {
    const shared = new SharedAtlases<Pages>();
    const a = shared.acquire("a", () => new Pages(), makeAtlas);
    const b = shared.acquire("b", () => new Pages(), makeAtlas);
    shared.clear();
    expect([a.pages.disposed, b.pages.disposed]).toEqual([1, 1]);
    expect(shared.size).toBe(0);
    // A user that lets go afterwards finds nothing left to free.
    shared.release("a");
    expect(a.pages.disposed).toBe(1);
  });
});

/** As much of a WebGPU device as sharing atlases touches, counting what is alive. */
class FakeDevice {
  textures = 0;
  buffers = 0;
  destroyed = false;
  readonly lost = new Promise<GPUDeviceLostInfo>(() => {});
  readonly queue = { writeTexture() {}, writeBuffer() {}, submit() {} };

  createBindGroupLayout(): object {
    return {};
  }
  createShaderModule(): object {
    return {};
  }
  createPipelineLayout(): object {
    return {};
  }
  createRenderPipeline(): object {
    return {};
  }
  createCommandEncoder(): object {
    return { copyTextureToTexture() {}, finish: () => ({}) };
  }
  createTexture(): object {
    this.textures++;
    let alive = true;
    return {
      createView: () => ({}),
      destroy: () => {
        if (alive) this.textures--;
        alive = false;
      },
    };
  }
  createBuffer(): object {
    this.buffers++;
    let alive = true;
    return {
      destroy: () => {
        if (alive) this.buffers--;
        alive = false;
      },
    };
  }
  destroy(): void {
    this.destroyed = true;
  }
}

/** Stands in for the browser's class of the same name, which the backend checks for. */
class FakeCanvasContext {
  configured = false;

  configure(): void {
    this.configured = true;
  }
  unconfigure(): void {
    this.configured = false;
  }
}

function canvas(): HTMLCanvasElement {
  const context = new FakeCanvasContext();
  return { width: 0, height: 0, getContext: () => context } as unknown as HTMLCanvasElement;
}

describe("the WebGPU tier's atlases", () => {
  const globals = globalThis as { GPUCanvasContext?: unknown };
  let before: unknown;

  beforeEach(() => {
    before = globals.GPUCanvasContext;
    globals.GPUCanvasContext = FakeCanvasContext;
  });

  afterEach(() => {
    globals.GPUCanvasContext = before;
  });

  function setup() {
    const device = new FakeDevice();
    const shared = new GpuShared(device as unknown as GPUDevice, "bgra8unorm");
    const pane = () => new WebGpuBackend(shared, canvas());
    return { device, shared, pane };
  }

  it("are shared by panes with the same font and freed with the last of them", () => {
    const { device, shared, pane } = setup();
    const first = pane();
    const second = pane();
    const atlas = first.atlas("a", makeAtlas);
    expect(second.atlas("a", makeAtlas)).toBe(atlas);
    // Asking again takes no second share.
    expect(first.atlas("a", makeAtlas)).toBe(atlas);
    expect(shared.atlasCount).toBe(1);

    // A glyph needs a page, and a page is a texture on the device.
    atlas.glyph(0x41, false, false, 1);
    expect(device.textures).toBe(1);

    first.dispose();
    expect(shared.atlasCount).toBe(1);
    expect(device.textures).toBe(1);

    second.dispose();
    expect(shared.atlasCount).toBe(0);
    expect(device.textures).toBe(0);
    expect(device.buffers).toBe(0);
  });

  it("lets go of an atlas when its pane changes font", () => {
    const { device, shared, pane } = setup();
    const only = pane();
    const small = only.atlas("small", makeAtlas);
    small.glyph(0x41, false, false, 1);
    expect(device.textures).toBe(1);

    const large = only.atlas("large", makeAtlas);
    expect(large).not.toBe(small);
    expect(shared.atlasCount).toBe(1);
    expect(device.textures).toBe(0);

    // Going back makes the atlas anew: the one from before was freed.
    expect(only.atlas("small", makeAtlas)).not.toBe(small);
    expect(shared.atlasCount).toBe(1);
    only.dispose();
    expect(shared.atlasCount).toBe(0);
  });

  it("keeps an atlas another pane still draws from when one pane changes font", () => {
    const { device, shared, pane } = setup();
    const first = pane();
    const second = pane();
    const atlas = first.atlas("a", makeAtlas);
    second.atlas("a", makeAtlas);
    atlas.glyph(0x41, false, false, 1);

    second.atlas("b", makeAtlas);
    expect(shared.atlasCount).toBe(2);
    expect(device.textures).toBe(1);
    expect(first.atlas("a", makeAtlas)).toBe(atlas);

    first.dispose();
    second.dispose();
    expect(shared.atlasCount).toBe(0);
    expect(device.textures).toBe(0);
  });

  it("frees them all with the device, and a pane disposed afterwards finds nothing to free", () => {
    const { device, shared, pane } = setup();
    const only = pane();
    only.atlas("a", makeAtlas).glyph(0x41, false, false, 1);

    shared.dispose();
    expect(device.destroyed).toBe(true);
    expect(device.textures).toBe(0);
    expect(shared.atlasCount).toBe(0);
    only.dispose();
    expect(shared.atlasCount).toBe(0);
  });
});
