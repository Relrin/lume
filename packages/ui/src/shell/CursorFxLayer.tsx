/**
 * The layer the cursor's effects are drawn in: one element over the whole window,
 * handed to the effects while it is on the page.
 */

import { useEffect, useRef } from "react";

import { useShell } from "./context.ts";

export function CursorFxLayer() {
  const app = useShell();
  const element = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const container = element.current;
    if (!container) return;
    app.cursorFx.attach(container);
    return () => app.cursorFx.detach();
  }, [app]);
  return <div ref={element} className="cursor-fx" aria-hidden="true" />;
}
