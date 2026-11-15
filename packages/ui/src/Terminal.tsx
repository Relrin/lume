/**
 * The terminal's place in the component tree.
 *
 * This component renders a handful of elements once and hands them to the terminal view,
 * which works on them directly from then on. It holds no state and takes no props, so
 * nothing a terminal does can make it render again.
 */

import { useEffect, useRef } from "react";

import { Prompts } from "./Prompts.tsx";
import { prompts, terminalApp, type TerminalApp } from "./terminal/boot.ts";

export function Terminal() {
  const root = useRef<HTMLDivElement>(null);
  const surface = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const scrollThumb = useRef<HTMLDivElement>(null);
  const preedit = useRef<HTMLDivElement>(null);
  const status = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const elements = {
      root: root.current,
      surface: surface.current,
      input: input.current,
      scrollThumb: scrollThumb.current,
      preedit: preedit.current,
      status: status.current,
    };
    let mounted: TerminalApp | undefined;
    let cancelled = false;
    terminalApp().then(
      (app) => {
        const { root, surface, input, scrollThumb, preedit, status } = elements;
        if (cancelled || !root || !surface || !input || !scrollThumb || !preedit || !status) return;
        app.view.mount({ root, surface, input, scrollThumb, preedit, status });
        mounted = app;
        app.connect();
        app.view.focus();
      },
      (error: unknown) => {
        // Nothing can be drawn, so the reason is put where a terminal would say it.
        if (cancelled || !elements.status) return;
        const reason = error instanceof Error ? error.message : String(error);
        elements.status.textContent = `The terminal could not be started: ${reason}`;
        elements.status.hidden = false;
      },
    );
    return () => {
      cancelled = true;
      mounted?.view.unmount();
    };
  }, []);

  return (
    <div className="terminal" ref={root}>
      <div className="terminal-surface" ref={surface} />
      <textarea
        className="terminal-input"
        ref={input}
        aria-label="Terminal input"
        autoCapitalize="off"
        autoComplete="off"
        autoCorrect="off"
        spellCheck={false}
      />
      <div className="terminal-scrollbar">
        <div className="terminal-scroll-thumb" ref={scrollThumb} hidden />
      </div>
      <div className="terminal-preedit" ref={preedit} hidden />
      <div className="terminal-status" ref={status} hidden />
      <Prompts store={prompts} />
    </div>
  );
}
