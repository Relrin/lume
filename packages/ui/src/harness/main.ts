/**
 * The harness page: runs one test suite inside the real webview and reports the result.
 *
 * The application shows this page in place of the terminal when it is started to run a
 * suite. What the suites check can only be checked here: that the terminal engine
 * behaves in this webview as it does elsewhere, that each renderer tier this webview
 * offers draws the right picture, and how fast the whole of it is. The report goes to
 * the core, which prints it and exits with the verdict.
 *
 * Opened in a browser through the development server, the page takes the suite from
 * `?suite=` and shows the report instead, as far as a suite can run without a core.
 */

import { Vt } from "@lume/vt";
import wasmUrl from "@lume/vt/ghostty-vt.wasm?url";

import { checkFrameContract } from "../frame-contract.ts";
import { runBench } from "./bench.ts";
import { runChrome } from "./chrome.ts";
import { runCorpus } from "./corpus.ts";
import "./harness.css";
import { done, harnessInfo, type HarnessInfo } from "./host.ts";
import { runSnapshots } from "./snapshots.ts";

/** What every suite's report has: whether it passed. */
interface SuiteReport {
  readonly ok: boolean;
}

const status = document.getElementById("status");
const output = document.getElementById("report");

function progress(text: string): void {
  if (status) status.textContent = text;
}

async function runSuite(suite: string, info: HarnessInfo | null): Promise<SuiteReport> {
  checkFrameContract();
  // The window's own page starts its engine itself.
  if (suite === "chrome") return runChrome(progress);
  const vt = await Vt.instantiate(fetch(wasmUrl));
  switch (suite) {
    case "corpus":
      return runCorpus(vt, progress);
    case "snapshots":
      return runSnapshots(vt, info, progress);
    case "bench":
      if (!info) throw new Error("the benchmarks need the application's core");
      return runBench(vt, progress);
    default:
      throw new Error(`there is no suite called ${suite}`);
  }
}

async function main(): Promise<void> {
  const info = await harnessInfo();
  const suite = info?.harness ?? new URLSearchParams(window.location.search).get("suite") ?? "";
  const about = {
    suite,
    engine: info?.engine ?? "browser",
    engineVersion: info?.version ?? null,
    os: info?.os ?? null,
    arch: info?.arch ?? null,
    userAgent: navigator.userAgent,
  };
  let report: Record<string, unknown>;
  try {
    progress(`running ${suite}`);
    report = { ...about, ...(await runSuite(suite, info)) };
  } catch (error) {
    report = {
      ...about,
      ok: false,
      error: error instanceof Error ? (error.stack ?? error.message) : String(error),
    };
  }
  progress(report["ok"] === true ? `${suite} passed` : `${suite} failed`);
  if (output) output.textContent = JSON.stringify(report, null, 2);
  if (info) await done(report);
}

void main();
