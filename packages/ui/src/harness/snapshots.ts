/**
 * The snapshot suite: a fixed set of frames is drawn by every renderer tier this webview
 * offers, and the pictures are compared.
 *
 * Two comparisons are made. The tiers are compared with one another, pixel by pixel: they
 * are three ways of drawing the same thing and must agree. And each picture is compared
 * with a reference picture kept for this webview engine, which is what catches all tiers
 * going wrong together.
 *
 * Text is drawn by the system's rasteriser, so a reference belongs to one engine, and
 * even there the edges of letters shift by a level or two between machines and versions.
 * The comparison with a reference therefore looks at small blocks of pixels rather than
 * single ones: a letter that is missing, moved or miscoloured changes its block plainly,
 * while a softer or sharper edge does not.
 *
 * A frame with text the test font lacks (CJK, emoji) is drawn with whatever fonts the
 * machine has, so no reference could be kept for it: for such frames the tiers are only
 * compared with one another. The same goes for the full screens drawn for timing, whose
 * pictures are large and show nothing the smaller frames do not. An engine nobody has made references for reports that, and
 * leaves its pictures where they can be collected and looked at.
 *
 * The cursor's effects are drawn too, each frozen at one moment, and held to references
 * of their own.
 */

import { GLYPH_CLUSTER_TAG, type Highlights, type RenderFrame } from "@lume/renderer";
import { FIXED_FRAMES } from "@lume/renderer/testing";
import type { Vt } from "@lume/vt";

import { drawCursorFx } from "./fx-snapshots.ts";
import { type HarnessInfo, readReference, saveFile } from "./host.ts";
import {
  canvasOf,
  createPane,
  drawWhole,
  fit,
  loadTestFont,
  TIERS,
  type TierPane,
} from "./panes.ts";
import { compare, differingBlocks, fromPng, readPixels, toPng, type Difference } from "./pixels.ts";
import {
  COMPARE_TOLERANCE,
  REFERENCE_MAX_CELLS,
  REFERENCE_MAX_DIFFERENT,
  SNAPSHOT_PIXEL_RATIO,
} from "./tuning.ts";
import { terminalFrames } from "./workloads.ts";

/** How a frame's pictures stood against its reference. */
export type ReferenceVerdict =
  /** The pictures agree with the reference. */
  | "matched"
  /** A picture differs from the reference. */
  | "differs"
  /** There is no reference for this engine. The picture was saved as a candidate. */
  | "unreferenced"
  /** The reference was written from this run's picture. */
  | "written"
  /**
   * No reference is kept for the frame: it uses the machine's own fonts, or it is a full
   * screen drawn for timing.
   */
  | "not-kept";

/** How one frame fared. */
export interface FrameResult {
  readonly name: string;
  /** How each tier's picture differs from the first tier's. */
  readonly tiers: Record<string, Difference>;
  readonly reference: ReferenceVerdict;
  /** What is wrong with the frame, if anything. */
  readonly problems: readonly string[];
}

/** The suite's report. */
export interface SnapshotReport {
  readonly ok: boolean;
  /** The tiers that drew. */
  readonly tiers: readonly string[];
  /** Tiers the webview does not offer, and why. */
  readonly unavailable: Record<string, string>;
  /** Frames with no reference for this engine. */
  readonly unreferenced: number;
  readonly frames: readonly FrameResult[];
}

/** A named frame, and what to free when it has been drawn. */
interface Subject {
  readonly name: string;
  readonly frame: RenderFrame;
  /** The runs to highlight when drawing it, if it has any. */
  readonly highlights?: Highlights | undefined;
  free(): void;
}

/**
 * Whether a frame has text the test font has no glyphs for: the scripts of East Asia,
 * emoji, and the symbols around them. Such text is drawn with the machine's own fonts.
 */
function usesSystemFonts(frame: RenderFrame): boolean {
  const beyond = (codepoint: number) =>
    codepoint >= 0x2e80 || (codepoint >= 0x1100 && codepoint <= 0x11ff);
  for (const glyph of frame.glyph) {
    if (glyph === 0) continue;
    if (glyph >= GLYPH_CLUSTER_TAG) {
      const text = frame.clusters[glyph - GLYPH_CLUSTER_TAG] ?? "";
      for (const character of text) {
        if (beyond(character.codePointAt(0) ?? 0)) return true;
      }
    } else if (beyond(glyph)) {
      return true;
    }
  }
  return false;
}

/** The name of a frame as a file name. */
function fileName(frame: string): string {
  return `${frame.replace(/[^a-z0-9]+/gi, "-").replace(/^-|-$/g, "")}.png`;
}

/** The directory that holds an engine's references. */
function engineDirectory(info: HarnessInfo | null): string | null {
  return info ? info.engine.toLowerCase().replace(/[^a-z0-9]+/g, "") : null;
}

function subjects(vt: Vt): Subject[] {
  const fixed = FIXED_FRAMES.map(({ name, build, highlights }) => ({
    name,
    frame: build(),
    highlights: highlights?.(),
    free: () => undefined,
  }));
  const fed = Object.entries(terminalFrames(vt)).map(([name, open]) => {
    const source = open();
    return { name, frame: source.frame, free: () => source.free() };
  });
  return [...fixed, ...fed];
}

/** Draws every frame on every tier and reports how the pictures compare. */
export async function runSnapshots(
  vt: Vt,
  info: HarnessInfo | null,
  progress: (text: string) => void,
): Promise<SnapshotReport> {
  await loadTestFont();
  const stage = document.createElement("div");
  stage.className = "harness-stage";
  document.body.append(stage);

  const panes: TierPane[] = [];
  const unavailable: Record<string, string> = {};
  for (const tier of TIERS) {
    const surface = document.createElement("div");
    surface.className = "harness-surface";
    stage.append(surface);
    const pane = await createPane(tier, surface);
    if (pane.renderer) {
      panes.push(pane);
    } else {
      unavailable[tier] = pane.error ?? "unavailable";
      surface.remove();
    }
  }

  const engine = engineDirectory(info);
  const update = info?.update ?? false;
  const frames: FrameResult[] = [];

  /**
   * Holds `picture`, and the others drawn of the same thing, to the reference kept for
   * `subject`, or writes it as the reference when asked to.
   */
  const againstReference = async (
    subject: string,
    picture: ImageData,
    pictures: ReadonlyMap<string, ImageData>,
    problems: string[],
  ): Promise<ReferenceVerdict> => {
    if (!engine) return "not-kept";
    const name = `${engine}/${fileName(subject)}`;
    if (update) {
      await saveFile("references", name, await toPng(picture));
      return "written";
    }
    const stored = await readReference(name);
    if (!stored) {
      // Kept where it can be collected: this is what a reference would be made from.
      await saveFile("out", `references/${name}`, await toPng(picture)).catch(() => undefined);
      return "unreferenced";
    }
    const wanted = await fromPng(stored);
    let verdict: ReferenceVerdict = "matched";
    for (const [tier, drawn] of pictures) {
      const share = differingBlocks(drawn, wanted);
      if (share <= REFERENCE_MAX_DIFFERENT) continue;
      verdict = "differs";
      problems.push(
        share === 1 && (drawn.width !== wanted.width || drawn.height !== wanted.height)
          ? `${tier} drew ${drawn.width}x${drawn.height}, the reference is ${wanted.width}x${wanted.height}`
          : `${tier} differs from the reference in ${(share * 100).toFixed(2)}% of its blocks`,
      );
    }
    return verdict;
  };
  for (const subject of subjects(vt)) {
    progress(`drawing ${subject.name}`);
    const problems: string[] = [];
    const pictures = new Map<string, ImageData>();
    for (const pane of panes) {
      fit(pane, subject.frame, SNAPSHOT_PIXEL_RATIO);
      pane.renderer?.setHighlights(subject.highlights);
      drawWhole(pane, subject.frame);
      // Read in the task that drew: a GPU canvas keeps its picture only until then.
      const canvas = canvasOf(pane);
      if (canvas) pictures.set(pane.kind, readPixels(canvas));
      else problems.push(`${pane.kind} has no canvas to read`);
    }
    const kept =
      !usesSystemFonts(subject.frame) &&
      subject.frame.cols * subject.frame.rows <= REFERENCE_MAX_CELLS;
    subject.free();

    // The tiers against one another: the first that drew is what the rest are held to.
    const [first, ...rest] = [...pictures.entries()];
    const tiers: Record<string, Difference> = {};
    for (const [tier, picture] of rest) {
      const difference = compare(picture, first![1], COMPARE_TOLERANCE);
      tiers[tier] = difference;
      if (!difference.sameSize) {
        problems.push(`${tier} and ${first![0]} drew pictures of different sizes`);
      } else if (difference.beyondTolerance > 0) {
        problems.push(
          `${tier} differs from ${first![0]} in ${difference.beyondTolerance} of ${difference.pixels} pixels`,
        );
      }
    }

    const reference: ReferenceVerdict =
      kept && first
        ? await againstReference(subject.name, first[1], pictures, problems)
        : "not-kept";

    if (problems.length > 0) {
      // What was drawn, for whoever looks into it.
      for (const [tier, picture] of pictures) {
        const name = `failed/${tier}/${fileName(subject.name)}`;
        await saveFile("out", name, await toPng(picture)).catch(() => undefined);
      }
    }
    frames.push({ name: subject.name, tiers, reference, problems });
  }

  for (const pane of panes) {
    pane.renderer?.dispose();
    pane.host.dispose();
  }
  stage.remove();

  progress("drawing the cursor's effects");
  for (const effect of drawCursorFx(SNAPSHOT_PIXEL_RATIO)) {
    const problems = [...effect.problems];
    // The WebGL2 picture is the one a reference is kept of; Canvas 2D draws its glow
    // otherwise, and is held only to the checks made as it was drawn.
    const webgl2 = effect.pictures.get("webgl2");
    const reference = webgl2
      ? await againstReference(effect.name, webgl2, new Map([["webgl2", webgl2]]), problems)
      : "not-kept";
    if (problems.length > 0) {
      for (const [kind, picture] of effect.pictures) {
        const name = `failed/${kind}/${fileName(effect.name)}`;
        await saveFile("out", name, await toPng(picture)).catch(() => undefined);
      }
    }
    frames.push({ name: effect.name, tiers: {}, reference, problems });
  }

  const drew = panes.length > 0;
  return {
    // A webview with no tier at all draws nothing, and nothing is not a pass.
    ok: drew && frames.every((frame) => frame.problems.length === 0),
    tiers: panes.map((pane) => pane.kind),
    unavailable,
    unreferenced: frames.filter((frame) => frame.reference === "unreferenced").length,
    frames,
  };
}
