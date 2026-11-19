/**
 * Shows the question or notice a terminal has for the user, one at a time.
 *
 * Everything shown here that came from the clipboard or from a program is rendered as
 * text. Each prompt opens with its cautious answer focused, so a key pressed in passing
 * does not approve it.
 */

import { useEffect, useRef, useSyncExternalStore } from "react";

import { PluginPickDialog } from "./plugins/Pick.tsx";
import { closingText, closingTitle } from "./shell/closing.ts";
import type { PluginQuestion, Prompt, PromptStore } from "./terminal/prompts.ts";

/** A number for each prompt, so that each choice opens with nothing typed. */
const numbers = new WeakMap<Prompt, number>();
let counted = 0;

function numberOf(prompt: Prompt): number {
  let number = numbers.get(prompt);
  if (number === undefined) {
    number = ++counted;
    numbers.set(prompt, number);
  }
  return number;
}

export function Prompts({ store }: { store: PromptStore }) {
  const prompt = useSyncExternalStore(store.subscribe, store.current);
  if (!prompt) return null;
  if (prompt.kind === "pick") return <PluginPickDialog key={numberOf(prompt)} prompt={prompt} />;
  return <PromptDialog prompt={prompt} />;
}

/**
 * Text to be typed into a terminal with its keys shown: Enter as ⏎ before the line it
 * ends, Tab as ⇥, Escape as ␛, and any other control character as ^ and its letter, so
 * that what would run a command is seen to.
 */
export function visibleKeys(text: string): string {
  let out = "";
  const characters = [...text];
  characters.forEach((character, index) => {
    const code = character.charCodeAt(0);
    if (character === "\r" || character === "\n") {
      // A line ended with both is one Enter.
      if (character === "\n" && characters[index - 1] === "\r") return;
      out += "⏎\n";
    } else if (character === "\t") {
      out += "⇥";
    } else if (code === 0x1b) {
      out += "␛";
    } else if (code === 0x7f) {
      out += "^?";
    } else if (code < 0x20) {
      out += `^${String.fromCharCode(code + 64)}`;
    } else {
      out += character;
    }
  });
  return out;
}

/** The words of a permission, as the user is asked for it. */
function permissionWords(permission: string): string {
  return permission === "process.run" ? "run this command" : "type this";
}

/** A plugin's question, drawn under its name. */
function describePlugin(ask: PluginQuestion): Described {
  const from = <p className="prompt-from">From {ask.plugin.name}</p>;
  switch (ask.kind) {
    case "permission":
      return {
        title: `${ask.plugin.name} asks for a permission`,
        body: (
          <>
            <p className="prompt-what">{ask.what}</p>
            {ask.warning ? <p className="prompt-warning">{ask.warning}</p> : null}
            {ask.reason !== "" && (
              <blockquote className="prompt-quote">
                <span className="prompt-quote-who">{ask.plugin.name} says:</span>
                {`\u201c${ask.reason}\u201d`}
              </blockquote>
            )}
          </>
        ),
        proceed: "Allow",
        dismiss: "Don't allow",
      };
    case "each":
      return {
        title: `Allow ${ask.plugin.name} to ${permissionWords(ask.permission)}?`,
        body: (
          <>
            {ask.place ? <p>In {ask.place}:</p> : null}
            <pre className="prompt-preview">
              {ask.permission === "panes.write" ? visibleKeys(ask.text) : ask.text}
            </pre>
          </>
        ),
        proceed: ask.permission === "process.run" ? "Run" : "Type",
        dismiss: "Don't allow",
      };
    case "confirm":
      return {
        title: ask.title,
        body: (
          <>
            {from}
            <p>{ask.message}</p>
          </>
        ),
        proceed: ask.confirm,
        danger: ask.danger,
        dismiss: "Cancel",
      };
  }
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

function describe(prompt: Exclude<Prompt, { kind: "pick" }>): Described {
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
    case "plugin":
      return describePlugin(prompt.ask);
  }
}

function PromptDialog({ prompt }: { prompt: Exclude<Prompt, { kind: "pick" }> }) {
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
