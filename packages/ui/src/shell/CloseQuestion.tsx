/**
 * The question asked before closing something that still has programs at work in it.
 *
 * It hangs under the control the close was asked for from, the close button of a tab,
 * a pane or the window, with a nub pointing at it; a close asked for from the keyboard
 * or a menu, with no button on show, gets it near the top of the window instead. It
 * names what is at work: an agent waiting on the user first, then what is still
 * running. A name's Show puts the question away and shows that pane.
 *
 * Cancel has the keyboard to begin with, so a key pressed in passing closes nothing.
 */

import { useContext, useEffect, useLayoutEffect, useRef, useState } from "react";

import type { BusyPane, Closing } from "../terminal/prompts.ts";
import { closingText, closingTitle } from "./closing.ts";
import { ShellContext } from "./context.ts";
import { ArrowRightIcon } from "./icons.tsx";

/** How wide the question is, and how far it is kept from the window's edges. */
const WIDTH_PX = 300;
const EDGE_PX = 8;
/** How far below its control it hangs. */
const GAP_PX = 10;
/** How many of the busy panes it names; the rest are counted. */
const NAMED_MAX = 3;

/** What the button that goes ahead says. */
function proceedLabel(closing: Closing): string {
  switch (closing.what) {
    case "moved-group":
      return "Move group";
    case "tabs":
      return "Close tabs";
    default:
      return `Close ${closing.what}`;
  }
}

/** Where the question stands, and where its nub points, from its control. */
function placeOf(closing: Closing): { left: number; top: number; nub: number | null } {
  const anchor = closing.anchor;
  const width = typeof window === "undefined" ? 1280 : window.innerWidth;
  if (!anchor) {
    return { left: Math.max(EDGE_PX, (width - WIDTH_PX) / 2), top: 72, nub: null };
  }
  const centre = anchor.left + anchor.width / 2;
  const left = Math.max(EDGE_PX, Math.min(centre - WIDTH_PX / 2, width - WIDTH_PX - EDGE_PX));
  return { left, top: anchor.bottom + GAP_PX, nub: centre - left };
}

function BusyRow({ busy, show }: { busy: BusyPane; show: (pane: string) => void }) {
  return (
    <button
      type="button"
      className="close-why"
      aria-label={`${busy.who} ${busy.rest}. Show it`}
      onClick={() => show(busy.pane)}
    >
      {busy.waiting ? (
        <i className="close-why-wait" />
      ) : (
        <i className="busy-spinner" data-agent={busy.agent || undefined} />
      )}
      <span className="close-why-text">
        <b>{busy.who}</b>
        <span>{busy.rest}</span>
      </span>
      <span className="close-why-go">
        Show
        <ArrowRightIcon />
      </span>
    </button>
  );
}

/** The question about a close, answered by `answer`. */
export function CloseQuestion({
  closing,
  answer,
}: {
  closing: Closing;
  answer: (ok: boolean) => void;
}) {
  const app = useContext(ShellContext);
  const cautious = useRef<HTMLButtonElement>(null);
  const [place, setPlace] = useState(() => placeOf(closing));
  const title = closingTitle(closing);
  const busy = closing.busy ?? [];
  const named = busy.slice(0, NAMED_MAX);
  const more = Math.max(0, closing.running - named.length);

  useEffect(() => {
    // The keyboard starts on Cancel without its ring: the ring shows once the user moves it.
    cautious.current?.focus({ focusVisible: false } as FocusOptions);
  }, [closing]);

  // A window that changes size while the question is open keeps it under its control.
  useLayoutEffect(() => {
    const again = () => setPlace(placeOf(closing));
    window.addEventListener("resize", again);
    return () => window.removeEventListener("resize", again);
  }, [closing]);

  const show = (pane: string) => {
    answer(false);
    app?.client
      .layoutApply({ type: "focus", target: { type: "pane", pane } })
      .catch(() => undefined);
  };

  return (
    <div
      className="close-scrim"
      onPointerDown={(event) => {
        event.stopPropagation();
        if (event.target === event.currentTarget) answer(false);
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") answer(false);
        // Keys pressed here are for the question, not for the terminal behind it.
        event.stopPropagation();
      }}
    >
      <div
        className="close-question"
        role="alertdialog"
        aria-modal="true"
        aria-label={title}
        data-anchored={place.nub !== null || undefined}
        style={
          {
            left: `${Math.round(place.left)}px`,
            top: `${Math.round(place.top)}px`,
            "--nub": `${Math.round(place.nub ?? WIDTH_PX / 2)}px`,
          } as React.CSSProperties
        }
      >
        <h2>{title}</h2>
        {named.length === 0 ? (
          <p className="close-text">{closingText(closing)}</p>
        ) : (
          <div className="close-whys">
            {named.map((item) => (
              <BusyRow key={item.pane} busy={item} show={show} />
            ))}
            {more > 0 && (
              <p className="close-more">
                and {more} more {more === 1 ? "program" : "programs"} still running
              </p>
            )}
          </div>
        )}
        <div className="close-actions">
          <button type="button" ref={cautious} onClick={() => answer(false)}>
            Cancel
          </button>
          <button type="button" className="close-proceed" onClick={() => answer(true)}>
            {proceedLabel(closing)}
          </button>
        </div>
      </div>
    </div>
  );
}
