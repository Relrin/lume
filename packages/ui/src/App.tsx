import { useEffect, useState } from "react";

import {
  type GpuCheck,
  type ProbeReport,
  reportMissingHarness,
  runProbe,
  webviewInfo,
} from "./probe.ts";
import { shellApp, type ShellApp } from "./shell/boot/index.ts";
import { Shell } from "./shell/Shell.tsx";

/**
 * What the window shows. Inside the application it is the terminals. Started only to be
 * checked, or opened in a browser with no core behind it, it reports what the webview
 * can do instead.
 */
type Showing = "terminal" | "report";

export function App() {
  const [showing, setShowing] = useState<Showing | null>(null);

  useEffect(() => {
    let current = true;
    void webviewInfo().then((info) => {
      // Started to run a test suite, the app shows the harness page, not this one.
      // Being here means the build has no such page.
      if (info?.harness) {
        void reportMissingHarness(info.harness);
        return;
      }
      if (current) setShowing(info && !info.probe ? "terminal" : "report");
    });
    return () => {
      current = false;
    };
  }, []);

  if (showing === "terminal") return <Terminals />;
  if (showing === "report") return <WebviewReport />;
  return null;
}

/** The window's terminals, once the page has started; if it cannot start, the reason. */
function Terminals() {
  const [app, setApp] = useState<ShellApp | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    // The page is started once however often this mounts.
    shellApp().then(
      (started) => {
        if (current) setApp(started);
      },
      (error: unknown) => {
        if (current) setFailure(error instanceof Error ? error.message : String(error));
      },
    );
    return () => {
      current = false;
    };
  }, []);

  if (app) return <Shell app={app} />;
  if (failure === null) return null;
  return (
    <main className="start-failure" role="alert">
      <h1>Lume could not start</h1>
      <p>{failure}</p>
    </main>
  );
}

// The checks run once per page load, not once per mount: React's strict mode mounts twice
// in development, and the probe must not report (and exit the app) twice.
let probe: Promise<ProbeReport> | undefined;

function WebviewReport() {
  const [report, setReport] = useState<ProbeReport | null>(null);

  useEffect(() => {
    let current = true;
    probe ??= runProbe();
    void probe.then((result) => {
      if (current) setReport(result);
    });
    return () => {
      current = false;
    };
  }, []);

  return (
    <main className="hello">
      <h1>Lume</h1>
      <p className="lead">This window reports what the webview on this machine can do.</p>
      {report ? <Report report={report} /> : <p>Checking the webview…</p>}
      {/* A development page; the built app does not contain it. */}
      {import.meta.env.DEV && (
        <p>
          <a href="/lab.html">Renderer lab</a>
        </p>
      )}
    </main>
  );
}

function Report({ report }: { report: ProbeReport }) {
  const { webview, vt } = report;
  return (
    <dl className="facts">
      <Fact label="Webview engine">
        {webview ? `${webview.engine} ${webview.version ?? "(version unknown)"}` : "not in Tauri"}
      </Fact>
      <Fact label="Platform">{webview ? `${webview.os} ${webview.arch}` : "browser"}</Fact>
      <Fact label="WebGPU">
        <Check check={report.webgpu} />
      </Fact>
      <Fact label="WebGL2">
        <Check check={report.webgl2} />
      </Fact>
      <Fact label="Canvas 2D">
        <Check check={report.canvas2d} />
      </Fact>
      <Fact label="libghostty-vt (WASM)">
        <span className={vt.ok ? "yes" : "no"}>{vt.ok ? "works" : "failed"}</span>
        {vt.version ? ` · ${vt.version}` : ""} · {vt.detail}
      </Fact>
      <Fact label="Device pixel ratio">{report.devicePixelRatio}</Fact>
      <Fact label="User agent">{report.userAgent}</Fact>
    </dl>
  );
}

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </>
  );
}

function Check({ check }: { check: GpuCheck }) {
  return (
    <>
      <span className={check.available ? "yes" : "no"}>
        {check.available ? "available" : "unavailable"}
      </span>{" "}
      · {check.detail}
    </>
  );
}
