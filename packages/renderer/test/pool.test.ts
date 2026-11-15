import { describe, expect, it } from "vitest";

import type { GlyphAtlas } from "../src/atlas/atlas.ts";
import { SharedAtlases } from "../src/atlas/shared.ts";
import { ATLAS_PAGE_SIZE, WEBGL2_CONTEXTS_MAX } from "../src/constants.ts";
import { RenderPool, type TierLimits, type TierMakers } from "../src/pool.ts";
import { TierUnavailableError, type Surface } from "../src/select.ts";
import {
  FakeFonts,
  makeFrame,
  putText,
  RecordingBackend,
  RecordingSink,
  TEST_THEME,
} from "../src/testing/index.ts";
import type { AtlasSink, CellRenderer, Size, TerminalTheme, Tier } from "../src/types.ts";

/** A canvas as far as a pool cares: something with an identity. */
interface FakeCanvas {
  id: number;
}

class FakeSurface implements Surface<FakeCanvas> {
  created = 0;

  createCanvas(): FakeCanvas {
    return { id: ++this.created };
  }
  show(): void {}
  remove(): void {}
  setCssSize(): void {}
}

const PAGE_BYTES = ATLAS_PAGE_SIZE * ATLAS_PAGE_SIZE;
const SIZE: Size = { width: 320, height: 160 };

/** A pool over recording backends, with the tiers in `unavailable` refusing to start. */
function pool(unavailable: Tier[] = [], limits?: TierLimits) {
  const made: Record<Tier, RecordingBackend[]> = { webgpu: [], webgl2: [], canvas2d: [] };
  const maker = (tier: Tier) => () => {
    if (unavailable.includes(tier)) throw new TierUnavailableError(tier, "test");
    const backend = new RecordingBackend(tier);
    made[tier].push(backend);
    return backend;
  };
  const makers: TierMakers<FakeCanvas> = {
    webgpu: maker("webgpu"),
    webgl2: maker("webgl2"),
    canvas2d: maker("canvas2d"),
  };
  const fonts = new FakeFonts();
  const order: Tier[] = ["webgpu", "webgl2", "canvas2d"];
  return { made, fonts, renderers: new RenderPool(order, makers, fonts, limits) };
}

function frame(text = "hello") {
  const f = makeFrame(40, 10);
  putText(f, 0, 0, text);
  return f;
}

/** Makes a renderer with a size, so that it can draw. */
function sized(renderers: RenderPool<FakeCanvas>, theme: TerminalTheme = TEST_THEME): CellRenderer {
  const renderer = renderers.createRenderer(new FakeSurface(), theme);
  renderer.resize(SIZE, 1);
  return renderer;
}

describe("the limit on WebGL2 contexts", () => {
  it("gives the pane that would be one too many the next tier", () => {
    const { renderers } = pool(["webgpu"]);
    const panes = Array.from({ length: WEBGL2_CONTEXTS_MAX }, () => sized(renderers));
    expect(panes.every((pane) => pane.tier === "webgl2")).toBe(true);

    const extra = sized(renderers);
    expect(extra.tier).toBe("canvas2d");
    expect(renderers.stats().webgl2Contexts).toBe(WEBGL2_CONTEXTS_MAX);
    expect(renderers.stats().renderers).toEqual({
      webgpu: 0,
      webgl2: WEBGL2_CONTEXTS_MAX,
      canvas2d: 1,
    });
  });

  it("has room again once a renderer is disposed", () => {
    const { renderers } = pool(["webgpu"], { webgl2: 2 });
    const first = sized(renderers);
    sized(renderers);
    expect(sized(renderers).tier).toBe("canvas2d");

    first.dispose();
    expect(renderers.stats().webgl2Contexts).toBe(1);
    expect(sized(renderers).tier).toBe("webgl2");
  });

  it("stops counting a context the moment it is lost", () => {
    const { renderers, made } = pool(["webgpu"], { webgl2: 1 });
    const pane = sized(renderers);
    made.webgl2[0]!.lose("context lost");

    // The pane has not drawn again yet, so it still stands on the lost tier.
    expect(renderers.stats().webgl2Contexts).toBe(0);
    expect(renderers.stats().renderers.webgl2).toBe(1);
    expect(sized(renderers).tier).toBe("webgl2");

    // Its next frame moves it on, and the lost backend is let go of.
    pane.draw(frame());
    expect(pane.tier).toBe("canvas2d");
    expect(made.webgl2[0]!.disposed).toBe(true);
    expect(renderers.stats().renderers).toEqual({ webgpu: 0, webgl2: 1, canvas2d: 1 });
  });

  it("still tells the renderer of the loss", () => {
    const { renderers, made } = pool(["webgpu"]);
    let asked = 0;
    const pane = renderers.createRenderer(new FakeSurface(), TEST_THEME, {
      onInvalidate: () => asked++,
    });
    pane.resize(SIZE, 1);
    made.webgl2[0]!.lose();
    expect(asked).toBe(1);
  });

  it("cannot make a renderer that may use WebGL2 only when there is no room", () => {
    const { renderers } = pool([], { webgl2: 1 });
    renderers.createRenderer(new FakeSurface(), TEST_THEME, { tier: "webgl2" });
    expect(() =>
      renderers.createRenderer(new FakeSurface(), TEST_THEME, { tier: "webgl2" }),
    ).toThrow(/no renderer tier could be started/);
  });

  it("leaves the other tiers without a limit", () => {
    const { renderers } = pool(["webgl2"]);
    const panes = Array.from({ length: WEBGL2_CONTEXTS_MAX * 2 }, () => sized(renderers));
    expect(panes.every((pane) => pane.tier === "webgpu")).toBe(true);
  });
});

describe("measuring cells without a renderer", () => {
  it("gives what a renderer with the same theme and pixel ratio reports", () => {
    const { renderers } = pool();
    const theme: TerminalTheme = {
      ...TEST_THEME,
      font: { family: "Test Mono", sizePx: 17, lineHeight: 1.4 },
      padding: 5,
    };
    for (const dpr of [1, 1.5, 2]) {
      const renderer = renderers.createRenderer(new FakeSurface(), theme);
      renderer.resize(SIZE, dpr);
      expect(renderers.measureCells(theme, dpr)).toEqual(renderer.metrics);
      renderer.dispose();
    }
  });

  it("makes no renderer, no canvas and no atlas", () => {
    const { renderers, made, fonts } = pool();
    const metrics = renderers.measureCells(TEST_THEME, 1);
    expect(metrics.cellWidth).toBe(8);
    expect(metrics.cellHeight).toBe(16);
    expect(made.webgpu).toHaveLength(0);
    expect(fonts.rasterisers).toHaveLength(0);
    expect(renderers.stats().atlases).toBe(0);
  });
});

/** A sink that can give its pages up, as the pages of a shared atlas can. */
class SharedSink extends RecordingSink {
  disposed = false;

  dispose(): void {
    this.disposed = true;
  }
}

/** A backend that takes its atlas from a store all backends of its tier share. */
class SharingBackend extends RecordingBackend {
  readonly #shared: SharedAtlases<SharedSink>;
  #key: string | undefined;
  #atlas: GlyphAtlas | undefined;

  constructor(shared: SharedAtlases<SharedSink>) {
    super("webgpu");
    this.#shared = shared;
  }

  override atlas(key: string, create: (sink: AtlasSink) => GlyphAtlas): GlyphAtlas {
    if (this.#atlas && this.#key === key) return this.#atlas;
    const { atlas } = this.#shared.acquire(key, () => new SharedSink(), create);
    if (this.#key !== undefined) this.#shared.release(this.#key);
    this.#key = key;
    this.#atlas = atlas;
    return atlas;
  }

  override dispose(): void {
    if (this.#key !== undefined) this.#shared.release(this.#key);
    this.#key = undefined;
    super.dispose();
  }
}

describe("what a window's renderers hold", () => {
  it("counts renderers by tier as they come and go", () => {
    const { renderers } = pool(["webgpu"], { webgl2: 1 });
    expect(renderers.stats()).toEqual({
      renderers: { webgpu: 0, webgl2: 0, canvas2d: 0 },
      webgl2Contexts: 0,
      atlases: 0,
      atlasBytes: 0,
    });

    const first = sized(renderers);
    const second = sized(renderers);
    expect(renderers.stats().renderers).toEqual({ webgpu: 0, webgl2: 1, canvas2d: 1 });

    first.dispose();
    second.dispose();
    // Disposing twice counts once.
    second.dispose();
    expect(renderers.stats().renderers).toEqual({ webgpu: 0, webgl2: 0, canvas2d: 0 });
  });

  it("measures an atlas by its pages, a byte a pixel for masks and four for colour", () => {
    const { renderers } = pool(["webgpu"]);
    const pane = sized(renderers);
    // A renderer that has not drawn has an atlas with no pages yet.
    expect(pane.metrics.cellWidth).toBe(8);
    expect(renderers.stats()).toMatchObject({ atlases: 1, atlasBytes: 0 });

    pane.draw(frame("plain"));
    expect(renderers.stats().atlasBytes).toBe(PAGE_BYTES);

    pane.draw(frame("\u{1f600}"));
    expect(renderers.stats().atlasBytes).toBe(PAGE_BYTES + 4 * PAGE_BYTES);

    pane.dispose();
    expect(renderers.stats()).toMatchObject({ atlases: 0, atlasBytes: 0 });
  });

  it("counts an atlas of their own for each pane on a tier that shares nothing", () => {
    const { renderers } = pool(["webgpu"]);
    const panes = [sized(renderers), sized(renderers), sized(renderers)];
    for (const pane of panes) pane.draw(frame());
    expect(renderers.stats()).toMatchObject({ atlases: 3, atlasBytes: 3 * PAGE_BYTES });
  });

  it("counts an atlas panes share once, and frees it with its last pane", () => {
    const shared = new SharedAtlases<SharedSink>();
    const fonts = new FakeFonts();
    const never = () => {
      throw new TierUnavailableError("webgl2", "test");
    };
    const renderers = new RenderPool<FakeCanvas>(
      ["webgpu"],
      { webgpu: () => new SharingBackend(shared), webgl2: never, canvas2d: never },
      fonts,
    );
    const larger: TerminalTheme = { ...TEST_THEME, font: { family: "Test Mono", sizePx: 20 } };

    const first = sized(renderers);
    const second = sized(renderers);
    first.draw(frame());
    second.draw(frame());
    expect(shared.size).toBe(1);
    expect(renderers.stats()).toMatchObject({ atlases: 1, atlasBytes: PAGE_BYTES });

    // Another size is another atlas, and the pane lets go of the one it had.
    second.setTheme(larger);
    second.draw(frame());
    expect(shared.size).toBe(2);
    expect(renderers.stats()).toMatchObject({ atlases: 2, atlasBytes: 2 * PAGE_BYTES });

    first.dispose();
    expect(shared.size).toBe(1);
    expect(renderers.stats()).toMatchObject({ atlases: 1, atlasBytes: PAGE_BYTES });

    second.dispose();
    expect(shared.size).toBe(0);
    expect(renderers.stats()).toMatchObject({ atlases: 0, atlasBytes: 0 });
  });
});
