/**
 * The page of the settings window that says what the application is: its version, the
 * version of the Rex server on this machine if one is installed, and the parts a
 * report of a fault is the better for naming.
 *
 * "Copy debug info" puts the same on the clipboard as text. It is made from versions
 * and nothing else (see `debug-info.ts`), so it names no host and holds no credential.
 */

import { useEffect, useState } from "react";

import type { AboutInfo } from "../core/types.ts";
import { sentence } from "../shell/notices.ts";
import type { SettingsApp } from "./boot.ts";
import { debugInfo, debugLines } from "./debug-info.ts";
import { Button } from "./rows.tsx";

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** What the application is, and the way to copy it. */
export function AboutPage({ app }: { app: SettingsApp }) {
  const [info, setInfo] = useState<AboutInfo | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  // Asked for each time the page is shown: a server may have been installed since.
  useEffect(() => {
    let current = true;
    app.client.aboutInfo().then(
      (told) => {
        if (current) setInfo(told);
      },
      (error: unknown) => {
        if (current) setFailure(sentence(reason(error)));
      },
    );
    return () => {
      current = false;
    };
  }, [app]);

  const copy = () => {
    if (!info) return;
    app.client.clipboardWrite(debugInfo(info)).then(
      () => app.notices.push("info", "Debug info copied."),
      (error: unknown) =>
        app.notices.push("error", `The debug info could not be copied: ${reason(error)}`),
    );
  };

  return (
    <div className="about-page">
      <header className="about-head">
        <h2 className="about-name">Lume</h2>
        <p className="about-version">{info ? `Version ${info.appVersion}` : " "}</p>
      </header>
      {failure !== null && <p className="settings-intro">{failure}</p>}
      {info && (
        <section className="settings-group">
          <div className="settings-rows">
            {/* The application's own version is in the heading above. */}
            {debugLines(info)
              .slice(1)
              .map(([name, value]) => (
                <div key={name} className="settings-row about-row">
                  <span className="about-label">{name}</span>
                  <span className="about-value">{value}</span>
                </div>
              ))}
          </div>
        </section>
      )}
      <div className="about-actions">
        <Button onPress={copy}>Copy debug info</Button>
        <span className="about-note">
          Versions only: no host names, folders or credentials are included.
        </span>
      </div>
      <p className="about-terms">
        Lume is a third-party client of the Rex server, not made or endorsed by Superlogical. “Rex”
        only names the server that Lume can connect to.
      </p>
    </div>
  );
}
