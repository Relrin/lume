/**
 * Shows the window's notices in a corner, newest at the bottom.
 *
 * A notice is text and a way to take it away. A warning or an error is announced at
 * once to whoever listens to the page; one that only informs waits its turn.
 */

import { useSyncExternalStore } from "react";

import { CloseIcon } from "./icons.tsx";
import type { NoticeStore, ShownNotice } from "./notices.ts";

function Notice({ notice, dismiss }: { notice: ShownNotice; dismiss: () => void }) {
  return (
    <div
      className="notice"
      data-level={notice.level}
      role={notice.level === "info" ? "status" : "alert"}
      onClick={dismiss}
    >
      <p className="notice-message">{notice.message}</p>
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

/** The notices on show, if there are any. */
export function Notices({ store }: { store: NoticeStore }) {
  const shown = useSyncExternalStore(store.subscribe, store.current);
  if (shown.length === 0) return null;
  return (
    <div className="notices">
      {shown.map((notice) => (
        <Notice key={notice.id} notice={notice} dismiss={() => store.dismiss(notice.id)} />
      ))}
    </div>
  );
}
