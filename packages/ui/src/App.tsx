import { useEffect, useState } from "react";

import { type GpuCheck, type ProbeReport, runProbe } from "./probe.ts";

// The checks run once per page load, not once per mount: React's strict mode mounts twice
// in development, and the probe must not report (and exit the app) twice.
let probe: Promise<ProbeReport> | undefined;

export function App() {
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
      <p className="lead">
        This window reports what the webview on this machine can do. There is no terminal yet.
      </p>
      {report ? <Report report={report} /> : <p>Checking the webview…</p>}
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
