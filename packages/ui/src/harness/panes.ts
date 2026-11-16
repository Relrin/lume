/**
 * Renderers for testing: one for each tier the webview offers, each held to that tier,
 * drawing with a font file that is the same on every machine.
 */

import {
  RenderHost,
  rgba,
  TIER_ORDER,
  type CellRenderer,
  type RendererOptions,
  type RenderFrame,
  type TerminalTheme,
  type Tier,
} from "@lume/renderer";

import fontUrl from "../../../../tests/fonts/JetBrainsMono-Regular.ttf?url";

/** A family name no installed font has, so the test font's file is the one measured. */
const FONT_FAMILY = "Lume Test Mono";

/**
 * The appearance tests draw with. What the test font lacks (CJK, emoji) falls to the
 * system's fonts, so frames with such text look different from machine to machine.
 */
export const TEST_THEME: TerminalTheme = {
  font: { family: `"${FONT_FAMILY}", monospace`, sizePx: 14 },
  cursor: rgba(0xf0, 0xf0, 0xf0),
  selectionBackground: rgba(0x2f, 0x4f, 0x8f),
  searchMatch: rgba(0xf0, 0xb3, 0x5a, 0x66),
  searchActive: rgba(0xf0, 0xb3, 0x5a),
  searchActiveText: rgba(0x16, 0x18, 0x1c),
  padding: 4,
};

/** Makes the test font available to the page. */
export async function loadTestFont(): Promise<void> {
  // Loaded from its bytes: fetching needs no more than the page's own origin.
  const bytes = await (await fetch(fontUrl)).arrayBuffer();
  const face = new FontFace(FONT_FAMILY, bytes);
  await face.load();
  document.fonts.add(face);
}

/** A renderer held to one tier, or the reason the webview does not offer that tier. */
export interface TierPane {
  /** The tier the pane is held to, or `"auto"` for the normal order with fallback. */
  readonly kind: Tier | "auto";
  readonly host: RenderHost;
  /** Where the renderer keeps its canvas. */
  readonly surface: HTMLElement;
  readonly renderer: CellRenderer | undefined;
  /** Why there is no renderer. */
  readonly error: string | undefined;
}

/**
 * Makes a pane for `kind` whose canvas goes into `surface`. A tier the webview lacks
 * gives a pane with no renderer and the reason.
 */
export async function createPane(
  kind: Tier | "auto",
  surface: HTMLElement,
  options: RendererOptions = {},
): Promise<TierPane> {
  const host = await RenderHost.create(kind === "auto" ? {} : { tiers: [kind] });
  try {
    const renderer = host.createRenderer(surface, TEST_THEME, options);
    return { kind, host, surface, renderer, error: undefined };
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    return { kind, host, surface, renderer: undefined, error: reason };
  }
}

/** The tiers in the order the renderer tries them. */
export const TIERS: readonly Tier[] = TIER_ORDER;

/** The canvas a renderer put into its pane. */
export function canvasOf(pane: TierPane): HTMLCanvasElement | undefined {
  return pane.surface.querySelector("canvas") ?? undefined;
}

/**
 * Sizes a pane's renderer so that the frame's grid fits exactly, at the given device
 * pixel ratio.
 */
export function fit(pane: TierPane, frame: RenderFrame, dpr: number): void {
  const renderer = pane.renderer;
  if (!renderer) return;
  // The metrics depend on the pixel ratio, so it is set before they are read.
  renderer.resize({ width: 1, height: 1 }, dpr);
  const m = renderer.metrics;
  renderer.resize(
    {
      width: (frame.cols * m.cellWidth + 2 * m.padding) / dpr,
      height: (frame.rows * m.cellHeight + 2 * m.padding) / dpr,
    },
    dpr,
  );
}

/** Draws the whole of `frame` on a pane, whatever an earlier draw left flagged. */
export function drawWhole(pane: TierPane, frame: RenderFrame): void {
  frame.dirty = 2;
  frame.dirtyRows.fill(1);
  pane.renderer?.draw(frame);
}
