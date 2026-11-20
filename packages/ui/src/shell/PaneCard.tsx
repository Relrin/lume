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

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type RefObject,
} from "react";
import { useStore } from "zustand";

import type { PaneDescription } from "../events/describe.ts";
import type { PaneIcon } from "../events/programs.ts";
import { FLOAT_EDGES } from "../layout/floats.ts";
import { selectFocus } from "../layout/selectors.ts";
import type { PaneContent, PaneId } from "../layout/types.ts";
import { DevBadge } from "../plugins/DevBadge.tsx";
import { PluginFrame } from "../plugins/frame/PluginFrame.tsx";
import { focusPaneFrame } from "../plugins/frame/registry.ts";
import { SearchBar } from "../search/SearchBar.tsx";
import { playSlot } from "../motion/slots.ts";
import { StatusSlotItems } from "../status/StatusItem.tsx";
import type { ShellApp } from "./boot.ts";
import { useShell } from "./context.ts";
import { describeNow, followPane, mark } from "./describing.ts";
import { useKeyHint } from "./hints.ts";
import { PluginViewPlaceholder } from "./PluginViewPlaceholder.tsx";
import { anchorOf } from "./tab-ui.ts";
import {
  AgainIcon,
  CloseIcon,
  DoneIcon,
  FloatIcon,
  ProgramTile,
  PutAwayIcon,
  SplitDownIcon,
  SplitRightIcon,
  ZoomIcon,
} from "./icons.tsx";

/**
 * The part of a header that renders: what changes seldom, such as a command starting or
 * a program saying it waits. Kept as one string so that what has not changed does not
 * render.
 */
interface HeaderMarks {
  readonly icon: PaneIcon;
  readonly running: boolean;
  readonly failed: number | null;
  readonly status: PaneDescription["status"];
  readonly ended: PaneDescription["ended"];
  readonly sizedElsewhere: boolean;
}

function marksOf(description: PaneDescription): string {
  const marks: HeaderMarks = {
    icon: description.icon,
    running: description.running,
    failed: description.failed,
    status: description.status,
    ended: description.ended,
    sizedElsewhere: description.sizedElsewhere,
  };
  return JSON.stringify(marks);
}

/**
 * Keeps what a header shows of a pane up to date: its name, where it is and what it has
 * gathered are written to their elements; the rest, which changes seldom, comes back to
 * be rendered.
 */
function usePaneHeader(
  app: ShellApp,
  pane: PaneId,
  header: RefObject<HTMLElement | null>,
  name: RefObject<HTMLElement | null>,
  where: RefObject<HTMLElement | null>,
): HeaderMarks {
  const [marks, setMarks] = useState(() => marksOf(describeNow(app, pane)));
  useLayoutEffect(
    () =>
      followPane(app, pane, (description) => {
        // Set as text: all of it comes from a program, and is never markup.
        writeText(name.current, description.name);
        const place = where.current;
        if (place) {
          writeText(place, description.where);
          place.title = description.whereInFull;
          place.hidden = description.where === "";
        }
        const element = header.current;
        if (element) {
          mark(element, "paneKind", description.kind, "shell");
          mark(element, "activity", description.activity, "idle");
        }
        setMarks(marksOf(description));
      }),
    [app, pane, header, name, where],
  );
  return JSON.parse(marks) as HeaderMarks;
}

/** Gives an element `text`, unless it has it already. */
function writeText(element: HTMLElement | null, text: string): void {
  if (element && element.textContent !== text) element.textContent = text;
}

/**
 * Keeps an element's text what a pane is called, and `holder`'s marks what the pane
 * has gathered and whether a program in it waits on the user, without rendering.
 */
export function usePaneName(app: ShellApp, pane: PaneId, holder?: RefObject<HTMLElement | null>) {
  const name = useRef<HTMLSpanElement>(null);
  useLayoutEffect(
    () =>
      followPane(app, pane, (description) => {
        writeText(name.current, description.name);
        const element = holder?.current;
        if (!element) return;
        mark(element, "activity", description.activity, "idle");
        mark(element, "waiting", String(description.status?.state === "blocked"), "false");
      }),
    [app, pane, holder],
  );
  return name;
}

/** The picture for what runs in a pane, rendering only when it changes. */
export function useProgramIcon(app: ShellApp, pane: PaneId): PaneIcon {
  return useSyncExternalStore(
    useCallback((changed) => followPane(app, pane, changed), [app, pane]),
    () => describeNow(app, pane).icon,
  );
}

/** The marks a header shows after its spacer: what runs, how the program says it stands. */
function HeaderMarksView({ pane, marks }: { pane: PaneId; marks: HeaderMarks }) {
  const app = useShell();
  const { running, failed, status, ended, sizedElsewhere } = marks;
  return (
    <>
      {sizedElsewhere && (
        <span
          className="pane-sized"
          title="Another view of this session decides its size. Click in this pane to give the terminal its size."
        >
          Sized by another view
        </span>
      )}
      {running && <span className="pane-running" role="img" aria-label="Running" title="Running" />}
      {failed !== null && (
        <span className="pane-exit" title={`The last command ended with code ${failed}`}>
          exit {failed}
        </span>
      )}
      {status && (
        <span
          className="pane-status"
          data-program-status={status.state}
          role="img"
          aria-label={status.label}
          title={status.tooltip ?? status.label}
        >
          <span className="pane-status-mark" aria-hidden="true">
            {status.state === "done" && <DoneIcon size={9} />}
          </span>
          <span className="pane-status-label" aria-hidden="true">
            {status.label}
          </span>
        </span>
      )}
      {ended && (
        <span className="pane-ended" data-ended={ended.again} title={ended.detail ?? ended.reason}>
          <span className="pane-ended-reason">{ended.reason}</span>
          <button
            type="button"
            className="control pane-again"
            tabIndex={-1}
            onPointerDown={(event) => event.preventDefault()}
            onClick={() => app.commands.run("lume.pane.reconnect", { pane })}
          >
            <AgainIcon />
            <span>{ended.again === "reconnect" ? "Reconnect" : "Restart"}</span>
          </button>
        </span>
      )}
    </>
  );
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

/**
 * A pane being renamed: a field in the place of its name. Left empty, it takes the
 * name away, and the pane is called after what runs in it again.
 */
function RenameField({ pane, shown }: { pane: PaneId; shown: string }) {
  const app = useShell();
  const field = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  const held = useRef("");
  useEffect(() => {
    const element = field.current;
    if (!element) return;
    held.current = app.layout.getState().panes[pane]?.title ?? "";
    element.value = held.current;
    element.focus();
    element.select();
  }, [app, pane]);
  const finish = (keep: boolean) => {
    if (done.current) return;
    done.current = true;
    const title = field.current?.value.trim() ?? "";
    if (keep && title !== held.current) {
      app.commands.run("lume.pane.rename", { pane, title: title === "" ? null : title });
    }
    app.tabUi.getState().endPaneRename();
    app.views.focus();
  };
  return (
    <input
      ref={field}
      className="pane-rename"
      aria-label="Pane name"
      placeholder={shown}
      spellCheck={false}
      autoComplete="off"
      // A press in the field is for the field: it does not move the card.
      onPointerDown={(event) => event.stopPropagation()}
      onDoubleClick={(event) => event.stopPropagation()}
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

function PaneHeader({
  pane,
  focused,
  floating,
  dev = false,
}: {
  pane: PaneId;
  focused: boolean;
  floating: boolean;
  /** Whether the pane shows the view of a plugin that is being worked on. */
  dev?: boolean;
}) {
  const app = useShell();
  const header = useRef<HTMLElement>(null);
  const name = useRef<HTMLSpanElement>(null);
  const where = useRef<HTMLSpanElement>(null);
  const marks = usePaneHeader(app, pane, header, name, where);
  const renaming = useStore(app.tabUi, (state) => state.renamingPane === pane);
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
      ref={header}
      className="pane-header"
      onPointerDown={(event) => {
        if (event.button !== 0 || (event.target as Element).closest("button, input")) return;
        // The keyboard stays with the terminal while its card is moved.
        event.preventDefault();
        if (floating) app.area.startFloatDrag(pane, "move", event.nativeEvent, event.currentTarget);
        else app.cardDrag.start(pane, event.nativeEvent, event.currentTarget);
      }}
      onDoubleClick={(event) => {
        // Pressed twice on its name, a pane is renamed there. The press is told by
        // where it was: while a card is moved its header is given every press on it.
        const rect = name.current?.getBoundingClientRect();
        if (!rect || rect.width === 0) return;
        const inside =
          event.clientX >= rect.left &&
          event.clientX <= rect.right &&
          event.clientY >= rect.top &&
          event.clientY <= rect.bottom;
        if (inside) app.tabUi.getState().startPaneRename(pane);
      }}
      onContextMenu={(event) => {
        // The field's own menu, to cut and paste with, is the system's.
        if ((event.target as Element).closest("input")) return;
        event.preventDefault();
        // A menu asked for from the keyboard comes with no place of the pointer's, and
        // opens under the header.
        const keyboard = event.clientX === 0 && event.clientY === 0;
        const anchor = keyboard
          ? anchorOf(event.currentTarget)
          : { left: event.clientX, top: event.clientY, bottom: event.clientY };
        app.tabUi.getState().open({ kind: "pane", pane, anchor, keyboard });
      }}
    >
      <span className="pane-icon">
        <ProgramTile tile={marks.icon} place="header" />
        <span className="pane-activity" aria-hidden="true" />
      </span>
      <span className="pane-title" ref={name} hidden={renaming} />
      {renaming && <RenameField pane={pane} shown={name.current?.textContent ?? ""} />}
      {dev && <DevBadge />}
      <span className="pane-cwd" ref={where} hidden />
      <span className="pane-spacer" />
      <HeaderMarksView pane={pane} marks={marks} />
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
 * edges to resize it by. `parked` says the pane is not on show: only the card of a
 * pane that shows a plugin's view is ever rendered so, to keep the view's frame for a
 * while after its tab was left.
 */
export function PaneCard({
  pane,
  header,
  floating = false,
  parked = false,
}: {
  pane: PaneId;
  header: boolean;
  floating?: boolean;
  parked?: boolean;
}) {
  const app = useShell();
  const plugin = useStore(app.layout, (state) => {
    const content = state.panes[pane]?.content;
    return content?.type === "plugin_view" ? content : undefined;
  });
  if (plugin !== undefined) {
    return (
      <PluginPaneCard
        pane={pane}
        header={header}
        floating={floating}
        parked={parked}
        content={plugin}
      />
    );
  }
  if (parked) return null;
  return <TerminalPaneCard pane={pane} header={header} floating={floating} />;
}

/** The content of a pane that shows a plugin's view. */
type PluginContent = Extract<PaneContent, { type: "plugin_view" }>;

/**
 * The card of a pane that shows one of a plugin's views: the header every pane has, and
 * the plugin's page in a frame where a terminal would be.
 *
 * A frame cannot be moved in the page without starting again, so the card of a pane
 * whose tab is not on show stays where it is, out of sight and out of reach, and its
 * frame with it.
 */
function PluginPaneCard({
  pane,
  header,
  floating,
  parked,
  content,
}: {
  pane: PaneId;
  header: boolean;
  floating: boolean;
  parked: boolean;
  content: PluginContent;
}) {
  const app = useShell();
  const card = useRef<HTMLElement>(null);
  const focused = useStore(app.layout, (state) => state.ready && selectFocus(state) === pane);
  const def = useStore(app.plugins, (state) =>
    state.shown?.pane_views.find(
      (one) => one.plugin === content.plugin && one.view === content.view,
    ),
  );
  const ready = useStore(app.plugins, (state) =>
    def === undefined
      ? false
      : (state.shown?.plugins.find((entry) => entry.id === def.plugin)?.ready ?? false),
  );
  const dev = useStore(
    app.plugins,
    (state) => state.shown?.plugins.find((entry) => entry.id === content.plugin)?.dev ?? false,
  );
  const headed = (header || floating) && !parked;
  useLayoutEffect(() => {
    const element = card.current;
    // A card that is not on show has no place among the tab's cards.
    if (!element || parked) return undefined;
    return app.area.registerPane(pane, element);
  }, [app, pane, parked]);
  /** Gives the pane the keyboard, in the layout and in its frame. */
  const take = () => {
    if (parked) return;
    if (!focused) {
      app.client
        .layoutApply({ type: "focus", target: { type: "pane", pane } })
        .catch(() => undefined);
    }
  };
  return (
    <section
      ref={card}
      className="pane-card"
      data-pane={pane}
      data-focused={focused}
      data-floating={floating}
      data-content="plugin"
      data-parked={parked}
      inert={parked}
      aria-hidden={parked || undefined}
      aria-label={`Pane ${pane}`}
      onPointerDown={() => {
        take();
        focusPaneFrame(pane);
      }}
    >
      {headed && <PaneHeader pane={pane} focused={focused} floating={floating} dev={dev} />}
      <div className="pane-plugin">
        {def !== undefined && ready ? (
          <PluginFrame
            plugin={def.plugin}
            view={def.view}
            place="pane"
            entry={def.entry}
            title={def.title}
            state={content.state}
            host={pane}
            shown={!parked}
            onFocusIn={take}
          />
        ) : (
          <PluginViewPlaceholder plugin={content.plugin} view={content.view} />
        )}
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

/** The card of a pane that shows a terminal. */
function TerminalPaneCard({
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
  const headed = header || floating;

  // A card that begins to float comes in as the settings choose.
  useLayoutEffect(() => {
    if (!floating) return undefined;
    const animation = playSlot("float.enter", card.current);
    return () => animation?.cancel();
  }, [floating]);

  // A card without a header says how its program ended on the terminal's own line,
  // since there is nowhere else to say it.
  useLayoutEffect(() => {
    if (headed) return undefined;
    let said = "";
    const stop = followPane(app, pane, (description) => {
      const ended = description.ended;
      const now = ended
        ? `${ended.reason}. Press Enter to ${ended.again === "reconnect" ? "reconnect" : "start it again"}.`
        : "";
      if (now === said) return;
      said = now;
      app.views.get(pane)?.showMessage(now);
    });
    return () => {
      stop();
      if (said !== "") app.views.get(pane)?.showMessage("");
    };
  }, [app, pane, headed]);

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
      {headed && <PaneHeader pane={pane} focused={focused} floating={floating} />}
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
