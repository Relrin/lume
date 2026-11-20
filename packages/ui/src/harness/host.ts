/**
 * The harness page's dealings with the core: what kind of run this is, reading reference
 * files, writing result files, and handing in the report.
 */

import { invoke } from "../transport/invoke.ts";

import { toCommandError } from "../transport/types.ts";

/** What the core says about the run. */
export interface HarnessInfo {
  /** `WebView2`, `WKWebView` or `WebKitGTK`. */
  readonly engine: string;
  readonly version: string | null;
  readonly os: string;
  readonly arch: string;
  /** The suite the application was started to run. */
  readonly harness: string | null;
  /** Whether reference files may be replaced. */
  readonly update: boolean;
}

/** The directories a suite may reach: what it produces, and what it compares against. */
export type HarnessRoot = "out" | "references";

/** What the core says about the run, or `null` when the page has no core behind it. */
export function harnessInfo(): Promise<HarnessInfo | null> {
  return invoke<HarnessInfo>("webview_info").catch(() => null);
}

/** Reads a reference file, or `null` when there is none of that name. */
export async function readReference(name: string): Promise<Uint8Array | null> {
  try {
    return new Uint8Array(await invoke<ArrayBuffer>("harness_read", { name }));
  } catch (thrown) {
    const error = toCommandError(thrown);
    // No file of that name, or no reference directory was named for this run.
    if (error.code === "not_found" || error.code === "unsupported") return null;
    throw error;
  }
}

/** Writes a file into one of the run's directories. */
export async function saveFile(root: HarnessRoot, name: string, bytes: Uint8Array): Promise<void> {
  try {
    await invoke("harness_save", bytes, { headers: { "lume-root": root, "lume-file": name } });
  } catch (thrown) {
    throw toCommandError(thrown);
  }
}

/** Hands in the report. The core prints it and ends the application. */
export async function done(report: unknown): Promise<void> {
  await invoke("harness_done", { report });
}
