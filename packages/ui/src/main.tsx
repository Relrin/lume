import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App.tsx";
import { ErrorBoundary } from "./ErrorBoundary.tsx";
import { startEarly } from "./shell/early.ts";
import { WindowFailed } from "./shell/WindowFailed.tsx";
import "./styles.css";

startEarly();

const root = document.getElementById("root");
if (!root) throw new Error("index.html has no #root element");

createRoot(root).render(
  <StrictMode>
    <ErrorBoundary
      fallback={(failure) => (
        <WindowFailed title="The window could not be drawn" failure={failure} />
      )}
    >
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
