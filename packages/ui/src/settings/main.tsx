import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";

import "../styles.css";
import { ErrorBoundary } from "../ErrorBoundary.tsx";
import { WindowFailed } from "../shell/WindowFailed.tsx";
import { settingsApp, type SettingsApp } from "./boot.ts";
import { SettingsWindow } from "./SettingsWindow.tsx";

/** The settings window, once its page has started; if it cannot start, the reason. */
function Settings() {
  const [app, setApp] = useState<SettingsApp | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    // The page is started once however often this mounts.
    settingsApp().then(
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

  if (app) return <SettingsWindow app={app} />;
  if (failure === null) return null;
  return (
    <main className="start-failure" role="alert">
      <h1>The settings could not be shown</h1>
      <p>{failure}</p>
    </main>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("settings.html has no #root element");

createRoot(root).render(
  <StrictMode>
    <ErrorBoundary
      fallback={(failure) => (
        <WindowFailed title="The settings could not be drawn" failure={failure} />
      )}
    >
      <Settings />
    </ErrorBoundary>
  </StrictMode>,
);
