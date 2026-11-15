/**
 * The window's tabs, in a row.
 *
 * Which tabs there are and which is on show come from the layout and render here. What
 * a tab is called does not: a tab without a name of its own is called after the program
 * in it, which may rename itself many times a second, so the label is written straight
 * to its element whenever it changes.
 */

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useStore } from "zustand";

import { selectActiveTab, selectTabOrder, tabTitle } from "../layout/selectors.ts";
import type { TabId } from "../layout/types.ts";
import { DRAG_START_PX } from "../tuning.ts";
import type { ShellApp } from "./boot.ts";
import { useShell } from "./context.ts";
import { useKeyHint } from "./hints.ts";
import { CloseIcon, PlusIcon, TerminalIcon } from "./icons.tsx";

/** Keeps an element's text the title of a tab, without rendering. */
function useTabLabel(app: ShellApp, tab: TabId) {
  const label = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const element = label.current;
    if (!element) return undefined;
    let shown: string | undefined;
    const write = () => {
      const title = tabTitle(app.layout.getState(), tab, app.titles.all());
      if (title === shown) return;
      shown = title;
      // Set as text: a title comes from a program and is never markup.
      element.textContent = title;
      element.parentElement?.setAttribute("title", title);
    };
    write();
    const stops = [app.layout.subscribe(write), app.titles.subscribe(write)];
    return () => stops.forEach((stop) => stop());
  }, [app, tab]);
  return label;
}

/** A tab being renamed: a field in the tab's place. */
function RenameField({ tab, done }: { tab: TabId; done: () => void }) {
  const app = useShell();
  const field = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const element = field.current;
    if (!element) return;
    element.value = app.layout.getState().tabs[tab]?.title ?? "";
    element.focus();
    element.select();
  }, [app, tab]);
  const finish = (keep: boolean) => {
    const title = field.current?.value.trim() ?? "";
    if (keep) app.commands.run("lume.tab.rename", { tab, title: title === "" ? null : title });
    done();
    app.views.focus();
  };
  return (
    <input
      ref={field}
      className="tab-rename"
      aria-label="Tab name"
      spellCheck={false}
      onBlur={() => finish(true)}
      onKeyDown={(event) => {
        if (event.key === "Enter") finish(true);
        else if (event.key === "Escape") finish(false);
        // What is typed here is the name, not a key for the shell.
        event.stopPropagation();
      }}
    />
  );
}

function TabItem({ tab, active, index }: { tab: TabId; active: boolean; index: number }) {
  const app = useShell();
  const label = useTabLabel(app, tab);
  const [renaming, setRenaming] = useState(false);
  const closeHint = useKeyHint("lume.tab.close");
  const drag = useRef<{ pointerId: number; x: number; moved: boolean } | null>(null);

  const show = () => {
    app.client
      .layoutApply({ type: "tab_activate", target: { type: "id", tab } })
      .catch(() => undefined);
  };
  /** The place among the tabs the pointer is over, by the middles of the tabs. */
  const placeAt = (x: number, strip: Element): number => {
    const tabs = [...strip.querySelectorAll<HTMLElement>("[data-tab]")];
    let place = 0;
    for (const element of tabs) {
      if (element.dataset["tab"] === tab) continue;
      const box = element.getBoundingClientRect();
      if (x > box.left + box.width / 2) place += 1;
    }
    return place;
  };

  return (
    <div
      className="tab"
      data-tab={tab}
      data-active={active}
      onPointerDown={(event) => {
        if (event.button !== 0 || renaming) return;
        if ((event.target as Element).closest(".tab-close")) return;
        drag.current = { pointerId: event.pointerId, x: event.clientX, moved: false };
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        const held = drag.current;
        if (!held || held.pointerId !== event.pointerId) return;
        if (Math.abs(event.clientX - held.x) >= DRAG_START_PX) held.moved = true;
        event.currentTarget.dataset["dragging"] = String(held.moved);
      }}
      onPointerUp={(event) => {
        const held = drag.current;
        drag.current = null;
        delete event.currentTarget.dataset["dragging"];
        if (!held || held.pointerId !== event.pointerId) return;
        const strip = event.currentTarget.parentElement;
        if (!held.moved || !strip) {
          show();
          app.views.focus();
          return;
        }
        const place = placeAt(event.clientX, strip);
        if (place !== index) {
          app.client.layoutApply({ type: "tab_move", tab, index: place }).catch(() => undefined);
        }
      }}
      onPointerCancel={(event) => {
        drag.current = null;
        delete event.currentTarget.dataset["dragging"];
      }}
      onDoubleClick={() => setRenaming(true)}
      onAuxClick={(event) => {
        // The middle button closes a tab, as it does wherever there are tabs.
        if (event.button === 1) app.commands.run("lume.tab.close", { tab });
      }}
    >
      {renaming ? (
        <RenameField tab={tab} done={() => setRenaming(false)} />
      ) : (
        <button
          type="button"
          role="tab"
          aria-selected={active}
          tabIndex={active ? 0 : -1}
          className="tab-label"
          onKeyDown={(event) => {
            if (event.key === "F2") setRenaming(true);
          }}
        >
          <span className="tab-icon">
            <TerminalIcon />
          </span>
          <span className="tab-title" ref={label} />
        </button>
      )}
      <button
        type="button"
        className="tab-close control"
        aria-label="Close tab"
        title={`Close tab${closeHint}`}
        tabIndex={-1}
        onClick={() => app.commands.run("lume.tab.close", { tab })}
      >
        <CloseIcon size={11} />
      </button>
    </div>
  );
}

/** The row of tabs and the button that opens another. */
export function TabStrip() {
  const app = useShell();
  const tabs = useStore(app.layout, (state) => (state.ready ? selectTabOrder(state) : NO_TABS));
  const active = useStore(app.layout, (state) => (state.ready ? selectActiveTab(state) : null));
  const newHint = useKeyHint("lume.tab.new");
  return (
    <div className="tab-strip">
      <div className="tab-list" role="tablist" aria-label="Tabs">
        {tabs.map((tab, index) => (
          <TabItem key={tab} tab={tab} active={tab === active} index={index} />
        ))}
      </div>
      <button
        type="button"
        className="control tab-new"
        aria-label="New tab"
        title={`New tab${newHint}`}
        onPointerDown={(event) => event.preventDefault()}
        onClick={() => app.commands.run("lume.tab.new")}
      >
        <PlusIcon />
      </button>
    </div>
  );
}

const NO_TABS: readonly TabId[] = [];
