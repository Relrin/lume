/**
 * Shows the window's notices in a corner, newest at the bottom.
 *
 * A notice is text and a way to take it away. A warning or an error is announced at
 * once to whoever listens to the page; one that only informs waits its turn. A notice
 * may offer one thing to do about what it says, such as undoing it: a button that runs
 * a command and takes the notice away.
 *
 * The stack, the time each notice stays, and taking one away with a swipe are sonner's.
 * What a notice looks like is the window's own: sonner is given the whole of each one
 * to draw, so a notice is styled by the theme's tokens like everything else here.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { toast, Toaster } from "sonner";

import { leaveSlot, useSlotMotion } from "../motion/slots.ts";
import { NOTICES_MAX } from "../tuning.ts";
import { CloseIcon } from "./icons.tsx";
import type { NoticeStore, ShownNotice } from "./notices.ts";

/** Runs the command a notice offers. */
export type RunAction = (command: string, args: unknown) => void;

/** The room between two notices, and between the notices and the window's edge. */
const GAP_PX = 8;
const EDGE_PX = 12;

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
  const element = useRef<HTMLDivElement>(null);
  useSlotMotion("toast.enter", element);
  const leave = () => {
    void leaveSlot("toast.exit", element.current).then(dismiss);
  };
  return (
    <div
      ref={element}
      className="notice"
      data-level={notice.level}
      role={notice.level === "info" ? "status" : "alert"}
      onClick={leave}
    >
      <p className="notice-message">{notice.message}</p>
      {action && (
        <button
          type="button"
          className="control notice-action"
          onClick={(event) => {
            event.stopPropagation();
            leave();
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
          leave();
        }}
      >
        <CloseIcon size={11} />
      </button>
    </div>
  );
}

/**
 * The notices on show. Without `run`, what a notice offers to do is not offered.
 * A page shows one of these: the notices of a page are one stack.
 */
export function Notices({ store, run }: { store: NoticeStore; run?: RunAction }) {
  // The newest `run` is the one a notice's button calls, without the notices on show
  // being handed over again each time the window is drawn.
  const running = useRef(run);
  running.current = run;
  const offered = run !== undefined;
  // The stack is put in the page with the first notice and stays from then on: a
  // window that has said nothing has nothing of it. A notice that arrives before the
  // stack is there waits here until it is.
  const [stacked, setStacked] = useState(false);
  const ready = useRef(false);
  const waiting = useRef<ShownNotice[]>([]);
  const show = useCallback(
    (notice: ShownNotice) => {
      toast.custom(
        (id) => (
          <Notice
            notice={notice}
            dismiss={() => toast.dismiss(id)}
            run={offered ? (command, args) => running.current?.(command, args) : undefined}
          />
        ),
        { id: notice.key, duration: notice.stays },
      );
    },
    [offered],
  );
  useEffect(
    () =>
      store.connect({
        show: (notice) => {
          if (ready.current) {
            show(notice);
            return;
          }
          waiting.current = [...waiting.current.filter((held) => held.key !== notice.key), notice];
          setStacked(true);
        },
        hide: (key) => {
          waiting.current = waiting.current.filter((held) => held.key !== key);
          toast.dismiss(key);
        },
      }),
    [store, show],
  );
  // Runs after the stack's own start, so the stack is listening for what is handed over.
  useEffect(() => {
    if (!stacked) return;
    ready.current = true;
    const waited = waiting.current;
    waiting.current = [];
    for (const notice of waited) show(notice);
  }, [stacked, show]);
  if (!stacked) return null;
  return (
    <Toaster
      className="notices"
      position="bottom-right"
      expand
      visibleToasts={NOTICES_MAX}
      gap={GAP_PX}
      offset={EDGE_PX}
      mobileOffset={EDGE_PX}
      containerAriaLabel="Notices"
      toastOptions={{ unstyled: true }}
    />
  );
}
