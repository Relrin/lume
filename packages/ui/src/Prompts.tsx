/**
 * Shows the question or notice a terminal has for the user, one at a time.
 *
 * Everything shown here that came from the clipboard or from a program is rendered as
 * text. Each prompt opens with its cautious answer focused, so a key pressed in passing
 * does not approve it.
 */

import { useEffect, useRef, useSyncExternalStore } from "react";

import { closingText, closingTitle } from "./shell/closing.ts";
import type { Prompt, PromptStore } from "./terminal/prompts.ts";

export function Prompts({ store }: { store: PromptStore }) {
  const prompt = useSyncExternalStore(store.subscribe, store.current);
  return prompt ? <PromptDialog prompt={prompt} /> : null;
}

interface Described {
  title: string;
  body: React.ReactNode;
  /** The label of the button that goes ahead, when there is something to go ahead with. */
  proceed?: string;
  /** Whether going ahead cannot be taken back, and the button says so by its look. */
  danger?: boolean;
  dismiss: string;
}

function describe(prompt: Prompt): Described {
  switch (prompt.kind) {
    case "paste": {
      const { lines, truncated, text } = prompt.preview;
      return {
        title: lines > 1 ? `Paste ${lines} lines?` : "Paste this text?",
        body: (
          <>
            <p>
              Text with line breaks or control characters can run commands as soon as it is pasted.
            </p>
            <pre className="prompt-preview">
              {text}
              {truncated ? "\n…" : ""}
            </pre>
          </>
        ),
        proceed: "Paste",
        dismiss: "Cancel",
      };
    }
    case "link":
      return {
        title: "Open this location?",
        body: <pre className="prompt-preview">{prompt.target}</pre>,
        proceed: "Open",
        dismiss: "Cancel",
      };
    case "close":
      return {
        title: closingTitle(prompt.closing),
        body: <p>{closingText(prompt.closing)}</p>,
        proceed: prompt.closing.what === "moved-group" ? "Move" : "Close",
        dismiss: "Cancel",
      };
    case "confirm":
      return {
        title: prompt.confirmation.title,
        body: <p>{prompt.confirmation.message}</p>,
        proceed: prompt.confirmation.confirm,
        danger: true,
        dismiss: "Cancel",
      };
    case "notice":
      return { title: "Lume", body: <p>{prompt.message}</p>, dismiss: "OK" };
  }
}

function PromptDialog({ prompt }: { prompt: Prompt }) {
  const { title, body, proceed, danger, dismiss } = describe(prompt);
  const cautious = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    cautious.current?.focus();
  }, [prompt]);

  return (
    <div
      className="prompt-backdrop"
      onKeyDown={(event) => {
        if (event.key === "Escape") prompt.answer(false);
        // Keys pressed here are for the dialog, not for the terminal behind it.
        event.stopPropagation();
      }}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <div className="prompt" role="alertdialog" aria-modal="true" aria-label={title}>
        <h2>{title}</h2>
        {body}
        <div className="prompt-actions">
          <button type="button" ref={cautious} onClick={() => prompt.answer(false)}>
            {dismiss}
          </button>
          {proceed && (
            <button
              type="button"
              className="prompt-proceed"
              data-danger={danger || undefined}
              onClick={() => prompt.answer(true)}
            >
              {proceed}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
