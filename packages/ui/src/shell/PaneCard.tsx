/**
 * One pane: a card with a header and a terminal.
 *
 * The card renders a handful of elements once and hands them over: its own element to
 * the controller that places the cards, and the terminal's elements to the pane's view,
 * which works on them directly from then on. Nothing a terminal does makes the card
 * render again, and neither does moving or resizing it. The bar a pane is searched
 * from lies over the top of its terminal and is rendered when it comes and goes.
 *
 * A card is moved by its header. A card in a tab is dragged to another place among the
 * tab's cards; a card that floats over the tabs is dragged wherever it is wanted, and
 * resized by its edges.
 */

import { useLayoutEffect, useRef } from "react";
import { useStore } from "zustand";

import { FLOAT_EDGES } from "../layout/floats.ts";
import { selectFocus } from "../layout/selectors.ts";
import type { PaneId } from "../layout/types.ts";
import { SearchBar } from "../search/SearchBar.tsx";
import { StatusSlotItems } from "../status/StatusItem.tsx";
import type { ShellApp } from "./boot.ts";
import { useShell } from "./context.ts";
import { useKeyHint } from "./hints.ts";
import {
  CloseIcon,
  FloatIcon,
  PutAwayIcon,
  SplitDownIcon,
  SplitRightIcon,
  TerminalIcon,
  ZoomIcon,
} from "./icons.tsx";

/** What a pane is called when neither the user nor its program has named it. */
const UNTITLED_PANE = "Terminal";

/** Keeps an element's text the title of a pane's program, without rendering. */
export function usePaneTitle(app: ShellApp, pane: PaneId) {
  const title = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const element = title.current;
    if (!element) return undefined;
    const write = () => {
      // A name the user gave the pane comes before the one its program gives itself.
      const named = app.layout.getState().panes[pane]?.title;
      // Set as text: a title comes from a program and is never markup.
      element.textContent = named ?? app.titles.get(pane) ?? UNTITLED_PANE;
    };
    write();
    const stops = [
      app.titles.subscribe((changed) => {
        if (changed === pane) write();
      }),
      app.layout.subscribe(write),
    ];
    return () => stops.forEach((stop) => stop());
  }, [app, pane]);
  return title;
}

function HeaderButton({
  label,
  hint,
  pressed,
  spare = false,
  lit = false,
  onPress,
  children,
}: {
  label: string;
  hint: string;
  pressed?: boolean;
  /** Whether a header too narrow for every button does without this one. */
  spare?: boolean;
  /** Whether the button, while pressed, is lit in the colour of a floating card's edge. */
  lit?: boolean;
  onPress: () => void;
  children: React.ReactNode;
}) {
  const classes = ["control", "pane-button"];
  if (spare) classes.push("pane-button-spare");
  if (lit) classes.push("pane-button-lit");
  return (
    <button
      type="button"
      className={classes.join(" ")}
      aria-label={label}
      aria-pressed={pressed}
      title={`${label}${hint}`}
      tabIndex={-1}
      // The keyboard stays with the terminal when a button is clicked.
      onPointerDown={(event) => event.preventDefault()}
      onClick={onPress}
    >
      {children}
    </button>
  );
}

function PaneHeader({
  pane,
  focused,
  floating,
}: {
  pane: PaneId;
  focused: boolean;
  floating: boolean;
}) {
  const app = useShell();
  const title = usePaneTitle(app, pane);
  const cwd = useStore(app.layout, (state) => {
    const content = state.panes[pane]?.content;
    return content?.type === "terminal" ? content.cwd : null;
  });
  const zoomed = useStore(
    app.layout,
    (state) => state.tabs[state.paneTabs[pane] ?? ""]?.zoomed === pane,
  );
  const hints = {
    right: useKeyHint("lume.pane.split-right"),
    down: useKeyHint("lume.pane.split-down"),
    float: useKeyHint("lume.pane.float"),
    minimise: useKeyHint("lume.pane.minimise"),
    zoom: useKeyHint("lume.pane.zoom"),
    close: useKeyHint("lume.pane.close"),
  };
  const run = (command: string) => () => app.commands.run(command, { pane });
  return (
    <header
      className="pane-header"
      onPointerDown={(event) => {
        if (event.button !== 0 || (event.target as Element).closest("button")) return;
        // The keyboard stays with the terminal while its card is moved.
        event.preventDefault();
        if (floating) app.area.startFloatDrag(pane, "move", event.nativeEvent, event.currentTarget);
        else app.cardDrag.start(pane, event.nativeEvent, event.currentTarget);
      }}
    >
      <span className="pane-icon">
        <TerminalIcon size={10} />
      </span>
      <span className="pane-title" ref={title} />
      {cwd !== null && (
        <span className="pane-cwd" title={cwd}>
          {cwd}
        </span>
      )}
      <span className="pane-spacer" />
      <StatusSlotItems store={app.status} slot="pane.header" pane={pane} />
      <span className="pane-buttons">
        {focused && !zoomed && !floating && (
          <>
            <HeaderButton
              label="Split right"
              hint={hints.right}
              spare
              onPress={run("lume.pane.split-right")}
            >
              <SplitRightIcon />
            </HeaderButton>
            <HeaderButton
              label="Split down"
              hint={hints.down}
              spare
              onPress={run("lume.pane.split-down")}
            >
              <SplitDownIcon />
            </HeaderButton>
          </>
        )}
        <HeaderButton
          label={floating ? "Dock back into the layout" : "Float above other panes"}
          hint={hints.float}
          pressed={floating}
          spare={!floating}
          lit
          onPress={run("lume.pane.float")}
        >
          <FloatIcon />
        </HeaderButton>
        {!floating && (
          <HeaderButton
            label={zoomed ? "Restore layout" : "Zoom pane"}
            hint={hints.zoom}
            pressed={zoomed}
            onPress={run("lume.pane.zoom")}
          >
            <ZoomIcon />
          </HeaderButton>
        )}
        <HeaderButton
          label="Minimise to the corner"
          hint={hints.minimise}
          spare={!floating}
          onPress={run("lume.pane.minimise")}
        >
          <PutAwayIcon />
        </HeaderButton>
        <HeaderButton label="Close pane" hint={hints.close} onPress={run("lume.pane.close")}>
          <CloseIcon />
        </HeaderButton>
      </span>
    </header>
  );
}

/**
 * A pane's card. `header` says whether the card has one; `floating`, whether the card
 * floats over the window's tabs, which gives it a header whatever else is said, and
 * edges to resize it by.
 */
export function PaneCard({
  pane,
  header,
  floating = false,
}: {
  pane: PaneId;
  header: boolean;
  floating?: boolean;
}) {
  const app = useShell();
  const card = useRef<HTMLElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const surface = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const scrollThumb = useRef<HTMLDivElement>(null);
  const preedit = useRef<HTMLDivElement>(null);
  const status = useRef<HTMLDivElement>(null);
  const focused = useStore(app.layout, (state) => state.ready && selectFocus(state) === pane);

  useLayoutEffect(() => {
    const elements = {
      root: root.current,
      surface: surface.current,
      input: input.current,
      scrollThumb: scrollThumb.current,
      preedit: preedit.current,
      status: status.current,
    };
    const element = card.current;
    if (!element || !elements.root || !elements.surface || !elements.input) return undefined;
    if (!elements.scrollThumb || !elements.preedit || !elements.status) return undefined;
    // The card has its place before the view measures what fits in it.
    const release = app.area.registerPane(pane, element);
    app.views.mount(pane, elements as Parameters<typeof app.views.mount>[1]);
    return () => {
      app.views.unmount(pane);
      release();
    };
    // A header that comes or goes changes the terminal's height, which the view sees
    // for itself; the elements stay the same.
  }, [app, pane]);

  return (
    <section
      ref={card}
      className="pane-card"
      data-pane={pane}
      data-focused={focused}
      data-floating={floating}
      aria-label={`Pane ${pane}`}
      onPointerDown={(event) => {
        // A press anywhere on a card gives its terminal the keyboard, except on the
        // search bar, which keeps it.
        if ((event.target as Element).closest("[data-search-bar]")) return;
        const view = app.views.get(pane);
        if (view && (!focused || !view.hasFocus)) view.focus();
      }}
    >
      {(header || floating) && <PaneHeader pane={pane} focused={focused} floating={floating} />}
      <SearchBar pane={pane} />
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
      </div>
      {floating &&
        FLOAT_EDGES.map((edge) => (
          <div
            key={edge}
            className="float-edge"
            data-edge={edge}
            onPointerDown={(event) => {
              if (event.button !== 0) return;
              event.preventDefault();
              app.area.startFloatDrag(pane, edge, event.nativeEvent, event.currentTarget);
            }}
          />
        ))}
    </section>
  );
}
