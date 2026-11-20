/**
 * The tools at the right end of the top bar, before the window's own buttons: the
 * Activity list, the command palette and a new tab.
 *
 * Activity lists the panes of the window that are doing something the user may want to
 * know about: a program waiting on them, a command or an agent still at work, something
 * that finished while they looked elsewhere. Its button carries a dot while something
 * new waits there, until the list is opened. A row shows its pane.
 */

import { useEffect, useRef, useState } from "react";

import type { PaneId } from "../layout/types.ts";
import type { ShellApp } from "./boot.ts";
import { useShell } from "./context.ts";
import { useKeyHint } from "./hints.ts";
import { AttentionIcon, CommandIcon, DoneIcon, EyeIcon, PlusIcon } from "./icons.tsx";
import { windowWork, type PaneWork } from "./work.ts";

/** How long ago something happened, or how long it has run, said shortly. */
export function shortSpan(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    const rest = seconds % 60;
    return rest === 0 || minutes >= 10 ? `${minutes}m` : `${minutes}m ${rest}s`;
  }
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

/** When each pane's work came to stand as it does, kept while the window runs. */
class Since {
  readonly #held = new Map<PaneId, { readonly key: string; readonly at: number }>();

  /** When `work` came to stand as it does now: the first time it was seen so. */
  of(work: PaneWork, now: number): number {
    const key = `${work.state}|${work.line}`;
    const held = this.#held.get(work.pane);
    if (held?.key === key) return held.at;
    this.#held.set(work.pane, { key, at: now });
    return now;
  }

  /** Forgets the panes that have nothing to show any more. */
  keep(panes: ReadonlySet<PaneId>): void {
    for (const pane of this.#held.keys()) if (!panes.has(pane)) this.#held.delete(pane);
  }
}

/** What a list of work says, as one string: the list is drawn again only when it changes. */
function signature(work: readonly PaneWork[]): string {
  return work
    .map((item) =>
      [item.pane, item.state, item.line, item.who, item.tabLabel, item.unread, item.agent].join(
        "|",
      ),
    )
    .join("\n");
}

/**
 * The work of the window's panes. It is read again when a pane's report or the layout
 * changes, at most once a frame, and the bar is drawn again only when what it lists
 * changed: a pane printing output changes nothing here.
 */
function useWindowWork(app: ShellApp): PaneWork[] {
  const [work, setWork] = useState<PaneWork[]>(() => windowWork(app));
  const shown = useRef(signature(work));
  useEffect(() => {
    let frame = 0;
    const read = () => {
      frame = 0;
      const next = windowWork(app);
      const said = signature(next);
      if (said === shown.current) return;
      shown.current = said;
      setWork(next);
    };
    const soon = () => {
      if (frame === 0) frame = requestAnimationFrame(read);
    };
    const stops = [app.reports.subscribeAll(soon), app.layout.subscribe(soon)];
    return () => {
      cancelAnimationFrame(frame);
      stops.forEach((stop) => stop());
    };
  }, [app]);
  return work;
}

/** The tile before a row of the Activity list, for how its work stands. */
function WorkTile({ work }: { work: PaneWork }) {
  return (
    <span className="activity-tile" data-state={work.state}>
      {work.state === "work" ? (
        <i className="busy-spinner" />
      ) : work.state === "wait" || work.state === "failed" ? (
        <AttentionIcon size={12} />
      ) : (
        <DoneIcon size={12} />
      )}
    </span>
  );
}

/** The Activity list, open under its button. */
function ActivityList({
  app,
  work,
  since,
  close,
}: {
  app: ShellApp;
  work: readonly PaneWork[];
  since: Since;
  close: () => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  // While the list is open the times in it move on.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  useEffect(() => {
    // The keyboard goes to the first row, so Enter shows it and Tab walks the rest.
    box.current?.querySelector<HTMLElement>(".activity-row")?.focus();
  }, []);

  const show = (item: PaneWork) => {
    close();
    app.client
      .layoutApply({ type: "focus", target: { type: "pane", pane: item.pane } })
      .catch(() => undefined);
  };
  const readAll = () => {
    for (const item of work) if (item.unread) app.reports.attend(item.pane);
  };

  return (
    <>
      <div className="activity-scrim" aria-hidden="true" onPointerDown={close} />
      <div
        ref={box}
        className="activity"
        role="dialog"
        aria-label="Activity"
        onKeyDown={(event) => {
          if (event.key !== "Escape") return;
          event.preventDefault();
          event.stopPropagation();
          close();
          app.views.focus();
        }}
      >
        <div className="activity-head">
          <span>Activity</span>
          <button
            type="button"
            className="activity-read"
            disabled={!work.some((item) => item.unread)}
            onClick={readAll}
          >
            Mark all read
          </button>
        </div>
        {work.length === 0 ? (
          <p className="activity-empty">Nothing is running or waiting on you.</p>
        ) : (
          work.map((item) => {
            const at = since.of(item, now);
            const when = item.state === "work" ? shortSpan(now - at) : `${shortSpan(now - at)} ago`;
            return (
              <button
                key={item.pane}
                type="button"
                className="activity-row"
                onClick={() => show(item)}
              >
                <WorkTile work={item} />
                <span className="activity-text">
                  <span className="activity-title">
                    <span>{item.tabLabel}</span>
                    {(item.unread || item.state === "wait") && <i className="activity-unread" />}
                  </span>
                  <span className="activity-line">{item.line}</span>
                </span>
                <span className="activity-when">{when}</span>
              </button>
            );
          })
        )}
        <p className="activity-foot">Tabs running a command or an agent show up here.</p>
      </div>
    </>
  );
}

/** The Activity button, the command palette's button and the new tab button. */
export function TopTools() {
  const app = useShell();
  const work = useWindowWork(app);
  const since = useRef(new Since()).current;
  const [open, setOpen] = useState(false);
  // What was there when the list was last opened has been seen.
  const [seenAt, setSeenAt] = useState(0);
  const paletteHint = useKeyHint("lume.palette.open");
  const newTabHint = useKeyHint("lume.tab.new");

  const now = Date.now();
  since.keep(new Set(work.map((item) => item.pane)));
  const news = work.filter(
    (item) => (item.unread || item.state === "wait") && since.of(item, now) > seenAt,
  ).length;
  const toggle = () => {
    setSeenAt(Date.now());
    setOpen(!open);
  };

  return (
    <div className="top-tools">
      <button
        type="button"
        className="control top-tool"
        aria-expanded={open}
        aria-haspopup="dialog"
        aria-label={news > 0 ? `Activity, ${news} new` : "Activity"}
        title="Activity"
        onPointerDown={(event) => event.preventDefault()}
        onClick={toggle}
      >
        <EyeIcon />
        {news > 0 && !open && <i className="top-tool-dot" />}
      </button>
      <button
        type="button"
        className="control top-tool"
        aria-label="Command palette"
        title={`Command palette${paletteHint}`}
        onPointerDown={(event) => event.preventDefault()}
        onClick={() => app.commands.run("lume.palette.open")}
      >
        <CommandIcon />
      </button>
      <button
        type="button"
        className="control top-tool"
        aria-label="New tab"
        title={`New tab${newTabHint}`}
        onPointerDown={(event) => event.preventDefault()}
        onClick={() => app.commands.run("lume.tab.new")}
      >
        <PlusIcon size={15} />
      </button>
      {open && (
        <ActivityList
          app={app}
          work={work}
          since={since}
          close={() => {
            setSeenAt(Date.now());
            setOpen(false);
          }}
        />
      )}
    </div>
  );
}
