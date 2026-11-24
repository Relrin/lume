/**
 * What the SSH client asks while a server is signed in to: a password, the passphrase of
 * a key, a code, or whether a machine is the one that is meant.
 *
 * The client's words are shown as text, apart from the rest: the machine that is signed
 * in to may have written them. What is typed is in the field and nowhere else of the
 * page, and the field is emptied when the dialog goes. Everything else of the window,
 * frames too, is out of reach meanwhile, so that nothing can take the keyboard while a
 * password is typed.
 */

import { useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { Glyph } from "./icons/Glyph.tsx";
import type { Prompt, SignInKind } from "./terminal/prompts.ts";

/** What differs from one kind of question to the next. */
interface Words {
  /** Said above the client's words, where they need it. */
  readonly lead: string | null;
  /** What the field is called, for a question that is answered by typing. */
  readonly field: string | null;
  readonly proceed: string;
  /** A second answer, beside going ahead: its label and what it answers. */
  readonly other: { readonly label: string; readonly answer: string } | null;
  /** What is said when it was asked before. */
  readonly again: string;
  /** What is said of the answer. */
  readonly note: string | null;
}

const FORGOTTEN = "Used for this connection, then forgotten.";
const NOT_ACCEPTED = "That was not accepted.";

const WORDS: Readonly<Record<SignInKind, Words>> = {
  password: {
    lead: null,
    field: "Password",
    proceed: "Sign in",
    other: null,
    again: NOT_ACCEPTED,
    note: FORGOTTEN,
  },
  passphrase: {
    lead: null,
    field: "Passphrase",
    proceed: "Unlock",
    other: { label: "Skip this key", answer: "" },
    again: "That was not accepted, or another machine on the way needs the same key.",
    note: FORGOTTEN,
  },
  code: {
    lead: null,
    field: "Answer",
    proceed: "Sign in",
    other: null,
    again: "It was asked again.",
    note: FORGOTTEN,
  },
  host_key: {
    lead: "SSH has not seen this machine before. Compare the fingerprint with one you trust.",
    field: null,
    proceed: "Trust and connect",
    other: null,
    again: "It was asked again.",
    note: null,
  },
  confirm: {
    lead: null,
    field: null,
    proceed: "Yes",
    other: { label: "No", answer: "no" },
    again: "It was asked again.",
    note: null,
  },
};

/** What going ahead answers when nothing is typed. */
const AGREED = "yes";

function Field({
  label,
  input,
  onTyped,
}: {
  label: string;
  input: React.RefObject<HTMLInputElement | null>;
  onTyped: (empty: boolean) => void;
}) {
  const [seen, setSeen] = useState(false);
  return (
    <div className="sign-in-field">
      <input
        ref={input}
        type={seen ? "text" : "password"}
        aria-label={label}
        autoComplete="off"
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        onInput={(event) => onTyped(event.currentTarget.value === "")}
      />
      <button
        type="button"
        aria-label={seen ? "Hide what is typed" : "Show what is typed"}
        aria-pressed={seen}
        title={seen ? "Hide" : "Show"}
        onClick={() => {
          setSeen(!seen);
          input.current?.focus();
        }}
      >
        <Glyph name={seen ? "eye-off" : "eye"} size={14} />
      </button>
    </div>
  );
}

function Line({ className, children }: { className: string; children: ReactNode }) {
  return <p className={className}>{children}</p>;
}

/** Drawn anew for each question, so that each begins with nothing typed. */
export function SignIn({ prompt }: { prompt: Extract<Prompt, { kind: "sign-in" }> }) {
  const { signIn } = prompt;
  const words = WORDS[signIn.kind];
  const dialog = useRef<HTMLDialogElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const cautious = useRef<HTMLButtonElement>(null);
  const [empty, setEmpty] = useState(true);
  // A code may be nothing: some machines only wait for Enter.
  const needed = words.field !== null && signIn.kind !== "code";

  // Before the dialog is drawn, and before it is taken away.
  useLayoutEffect(() => {
    const shown = dialog.current;
    const typed = input.current;
    shown?.showModal();
    (typed ?? cautious.current)?.focus();
    return () => {
      if (typed) typed.value = "";
      // Closed, and not only taken away: the keyboard goes back to what had it.
      shown?.close();
    };
  }, []);

  const cancel = () => prompt.answer(false);
  const proceed = () => {
    const answer = input.current?.value ?? AGREED;
    if (answer !== "" || !needed) prompt.answer(true, answer);
  };
  const onKey = (event: KeyboardEvent) => {
    // Keys pressed here are for the dialog, not for the terminal behind it.
    event.stopPropagation();
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      cancel();
    } else if (event.key === "Enter" && event.target === input.current) {
      event.preventDefault();
      proceed();
    }
  };
  const title = `Sign in to ${signIn.host}`;
  return (
    <dialog
      ref={dialog}
      className="prompt sign-in"
      role="alertdialog"
      aria-labelledby="sign-in-title"
      aria-describedby="sign-in-words"
      data-kind={signIn.kind}
      onKeyDown={onKey}
      onCancel={(event) => {
        event.preventDefault();
        cancel();
      }}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <div className="sign-in-head">
        <h2 id="sign-in-title">{title}</h2>
        {signIn.address !== signIn.host && (
          <Line className="sign-in-address">{signIn.address}</Line>
        )}
      </div>
      {words.lead !== null && <p>{words.lead}</p>}
      <div className="sign-in-asks">
        <span>SSH asks:</span>
        <pre className="prompt-preview" id="sign-in-words">
          {signIn.words}
        </pre>
      </div>
      {signIn.again && (
        <p className="prompt-warning" role="status">
          {words.again}
        </p>
      )}
      {words.field !== null && <Field label={words.field} input={input} onTyped={setEmpty} />}
      {words.note !== null && <Line className="sign-in-note">{words.note}</Line>}
      <div className="prompt-actions">
        <button type="button" ref={cautious} onClick={cancel}>
          Cancel
        </button>
        {words.other !== null && (
          <button type="button" onClick={() => prompt.answer(true, words.other?.answer ?? "")}>
            {words.other.label}
          </button>
        )}
        <button
          type="button"
          className="prompt-proceed"
          disabled={needed && empty}
          onClick={proceed}
        >
          {words.proceed}
        </button>
      </div>
    </dialog>
  );
}
