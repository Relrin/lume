/**
 * Checks the window runs inside the real webview: which engine it is, which GPU paths it
 * offers, and whether libghostty-vt works there.
 */

import { Vt } from "@lume/vt";
import wasmUrl from "@lume/vt/ghostty-vt.wasm?url";
import { invoke } from "./transport/invoke.ts";

export interface WebviewInfo {
  engine: string;
  version: string | null;
  os: string;
  arch: string;
  appVersion: string;
  probe: boolean;
  /** The test suite the app was started to run, if it was started for one. */
  harness: string | null;
}

export interface GpuCheck {
  available: boolean;
  detail: string;
}

export interface VtCheck {
  ok: boolean;
  version: string | null;
  detail: string;
}

export interface ProbeReport {
  ok: boolean;
  webview: WebviewInfo | null;
  userAgent: string;
  devicePixelRatio: number;
  webgpu: GpuCheck;
  webgl2: GpuCheck;
  canvas2d: GpuCheck;
  vt: VtCheck;
}

// The parts of the WebGPU API this file touches, to avoid a dependency for one call.
interface GpuAdapterInfoLike {
  vendor?: string;
  architecture?: string;
  device?: string;
  description?: string;
}
interface GpuAdapterLike {
  info?: GpuAdapterInfoLike;
  requestDevice(): Promise<{ destroy(): void }>;
}
interface GpuLike {
  requestAdapter(): Promise<GpuAdapterLike | null>;
}

async function checkWebGpu(): Promise<GpuCheck> {
  const gpu = (navigator as Navigator & { gpu?: GpuLike }).gpu;
  if (!gpu) return { available: false, detail: "navigator.gpu is not present" };
  try {
    const adapter = await gpu.requestAdapter();
    if (!adapter) return { available: false, detail: "no adapter was returned" };
    // An adapter alone is not enough: the renderer needs a device.
    const device = await adapter.requestDevice();
    device.destroy();
    const info = adapter.info ?? {};
    const parts = [info.vendor, info.architecture, info.device, info.description].filter(Boolean);
    return { available: true, detail: parts.length > 0 ? parts.join(" / ") : "adapter and device" };
  } catch (err) {
    return { available: false, detail: `request failed: ${String(err)}` };
  }
}

function checkWebGl2(): GpuCheck {
  try {
    const gl = document.createElement("canvas").getContext("webgl2");
    if (!gl) return { available: false, detail: "no webgl2 context" };
    const ext = gl.getExtension("WEBGL_debug_renderer_info");
    const renderer = ext
      ? String(gl.getParameter(ext.UNMASKED_RENDERER_WEBGL))
      : String(gl.getParameter(gl.RENDERER));
    gl.getExtension("WEBGL_lose_context")?.loseContext();
    return { available: true, detail: renderer };
  } catch (err) {
    return { available: false, detail: `failed: ${String(err)}` };
  }
}

function checkCanvas2d(): GpuCheck {
  const ctx = document.createElement("canvas").getContext("2d");
  return ctx
    ? { available: true, detail: "2d context" }
    : { available: false, detail: "no 2d context" };
}

/** The same smoke test the Rust and Node suites run, here inside the webview. */
async function checkVt(): Promise<VtCheck> {
  try {
    const vt = await Vt.instantiate(fetch(wasmUrl));
    const term = vt.createTerminal(80, 24);
    try {
      term.write("hello\r\n\x1b[31mred");
      const row0 = term.rowText(0);
      const fg = term.cell(0, 1).fg;
      const ok = row0 === "hello" && fg.kind === "palette" && fg.index === 1;
      return {
        ok,
        version: vt.version,
        detail: ok ? "smoke test passed" : `unexpected result: ${JSON.stringify({ row0, fg })}`,
      };
    } finally {
      term.free();
    }
  } catch (err) {
    return { ok: false, version: null, detail: `failed: ${String(err)}` };
  }
}

let info: Promise<WebviewInfo | null> | undefined;

/**
 * What the core says about the webview, or `null` when there is no core: the page was
 * opened in a browser. Asked once for each page.
 */
export function webviewInfo(): Promise<WebviewInfo | null> {
  // Not running inside Tauri (for example `vite` opened in a browser) fails the call.
  info ??= invoke<WebviewInfo>("webview_info").catch(() => null);
  return info;
}

/**
 * Says that the test suite the app was started for cannot run: this page is shown
 * where the harness page should be, which is what a build without that page serves.
 * The core prints the report and exits, so the run fails instead of opening a terminal.
 */
export async function reportMissingHarness(suite: string): Promise<void> {
  const report = {
    ok: false,
    suite,
    error: "this build of the application does not contain the harness page",
  };
  await invoke("harness_done", { report }).catch(() => undefined);
}

export async function runProbe(): Promise<ProbeReport> {
  const [webview, webgpu, vt] = await Promise.all([webviewInfo(), checkWebGpu(), checkVt()]);
  const webgl2 = checkWebGl2();
  const canvas2d = checkCanvas2d();
  const report: ProbeReport = {
    // The app is usable when the terminal engine runs and at least one renderer tier exists.
    ok: vt.ok && (webgpu.available || webgl2.available || canvas2d.available),
    webview,
    userAgent: navigator.userAgent,
    devicePixelRatio: window.devicePixelRatio,
    webgpu,
    webgl2,
    canvas2d,
    vt,
  };
  if (webview) {
    // In `--probe` mode the core prints the report and exits; otherwise this is ignored.
    await invoke("probe_done", { report }).catch(() => undefined);
  }
  return report;
}
