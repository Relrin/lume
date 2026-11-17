/**
 * Shows the window's notices in a corner, newest at the bottom.
 *
 * A notice is text and a way to take it away. A warning or an error is announced at
 * once to whoever listens to the page; one that only informs waits its turn. A notice
 * may offer one thing to do about what it says, such as undoing it: a button that runs
 * a command and takes the notice away.
 */

import { useSyncExternalStore } from "react";

import { CloseIcon } from "./icons.tsx";
import type { NoticeStore, ShownNotice } from "./notices.ts";

/** Runs the command a notice offers. */
export type RunAction = (command: string, args: unknown) => void;

function Notice({
  notice,
  dismiss,
  run,
}: {
  notice: ShownNotice;
  dismiss: () => void;
  run: RunAction | undefined;
}) {
  const action = run === undefined ? undefined : notice.action;
  return (
    <div
      className="notice"
      data-level={notice.level}
      role={notice.level === "info" ? "status" : "alert"}
      onClick={dismiss}
    >
      <p className="notice-message">{notice.message}</p>
      {action && (
        <button
          type="button"
          className="control notice-action"
          onClick={(event) => {
            event.stopPropagation();
            dismiss();
            run?.(action.command, action.args ?? null);
          }}
        >
          {action.label}
        </button>
      )}
      <button
        type="button"
        className="control notice-close"
        aria-label="Dismiss"
        title="Dismiss"
        onClick={(event) => {
          event.stopPropagation();
          dismiss();
        }}
      >
        <CloseIcon size={11} />
      </button>
    </div>
  );
}

/**
 * The notices on show, if there are any. Without `run`, what a notice offers to do is
 * not offered.
 */
export function Notices({ store, run }: { store: NoticeStore; run?: RunAction }) {
  const shown = useSyncExternalStore(store.subscribe, store.current);
  if (shown.length === 0) return null;
  return (
    <div className="notices">
      {shown.map((notice) => (
        <Notice
          key={notice.id}
          notice={notice}
          dismiss={() => store.dismiss(notice.id)}
          run={run}
        />
      ))}
    </div>
  );
}
