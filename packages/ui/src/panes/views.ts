/**
 * The terminals of this window's panes.
 *
 * Every terminal pane of the window has a view for as long as the pane exists, whether
 * or not its tab is on show. This follows the layout: a pane that appears gets a view,
 * a pane that goes has its view disposed of, and a view is attached to whatever session
 * the layout says its pane shows. A pane without a session has one started by the core,
 * at the grid the pane has.
 *
 * The scratch terminal is the one pane that leaves a window without closing: put away,
 * it is in no window. The window that showed it keeps its view meanwhile, so that what
 * was on its screen is there when it is shown again.
 *
 * A view out of sight can let go of its terminal after a while, when the configuration
 * says so; its session stays attached, and the screen comes back from the session's
 * backend when it is shown again.
 *
 * When the window is given another workspace, every view goes: the panes of two
 * workspaces may be called the same, and are never the same panes.
 *
 * Views are made and kept outside the component tree. A card that is rendered hands its
 * elements to its pane's view and takes them back when it goes; the view, its terminal
 * and its session carry on without them.
 */

import type { CoreClient } from "../core/client.ts";
import type { ConfigView } from "../core/types.ts";
import type { TabAreaController } from "../layout/controller.ts";
import { gridFor, shapeOf } from "../layout/estimate.ts";
import { computeRects, unitRectToPx } from "../layout/geometry.ts";
import { panesOf, selectFocus, selectWindowPanes } from "../layout/selectors.ts";
import type { LayoutState, LayoutStore } from "../layout/store.ts";
import type { PaneContent, PaneId } from "../layout/types.ts";
import {
  HIDDEN_RELEASE_DEFAULT_SECONDS,
  HISTORY_RELEASE_DEFAULT_SECONDS,
  SCROLLBACK_DEFAULT_LINES,
  WINDOW_HISTORY_DEFAULT_LINES,
} from "../limits.ts";
import { rendererTheme, type Appearance } from "../theme.ts";
import { CommandError, type TermSize } from "../transport/types.ts";
import { FONT_SIZE_MAX_PX, FONT_SIZE_MIN_PX } from "../tuning.ts";
import {
  TerminalView,
  type ViewElements,
  type ViewServices,
  type ViewSettings,
} from "../terminal/view.ts";
import type { PaneReports } from "../events/store.ts";
import type { PaneTitles } from "./titles.ts";

/** What every view of the window shares. */
export type SharedServices = Omit<
  ViewServices,
  | "shortcut"
  | "attached"
  | "reported"
  | "exited"
  | "sizedElsewhere"
  | "focused"
  | "restart"
  | "searched"
>;

/** What the views need from their surroundings. */
export interface ViewsServices {
  readonly shared: SharedServices;
  readonly layout: LayoutStore;
  readonly client: CoreClient;
  readonly area: TabAreaController;
  readonly titles: PaneTitles;
  /** What the panes' sessions have reported. */
  readonly reports: PaneReports;
  /** The configuration in use, or nothing before the core has sent one. */
  config(): ConfigView | null;
  /** The look the configuration comes to. */
  appearance(): Appearance;
  /** The look of a pane's terminal: the window's, or what the pane's profile asks for. */
  appearanceOf(pane: PaneId): Appearance;
  /** Offers a key pressed in a pane's terminal to the shell's own bindings. */
  shortcut(pane: PaneId, event: KeyboardEvent, alternateScreen: boolean): boolean;
  /** Told that a pane's terminal has taken the keyboard. */
  keyboard(pane: PaneId): void;
  /** Told that the search of a pane's terminal has caught up with the terminal. */
  searched(pane: PaneId): void;
}

/** One pane's view and what is known of its session. */
interface Entry {
  readonly view: TerminalView;
  /** The session the view shows, or is being attached to. */
  session: string | null;
  /** The core has been asked to start a session for the pane, or could not. */
  asked: boolean;
  /** Settles when the view has attached to its session, or could not. */
  attaching: Promise<void> | undefined;
  /** The look the view was last given. */
  look: Appearance | undefined;
}

/** Why a session cannot be attached to for good: it is gone, not busy. */
const GONE = new Set(["not_found", "session_exited", "forbidden"]);

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** The views of this window's terminal panes. */
export class PaneViews {
  readonly #services: ViewsServices;
  readonly #entries = new Map<PaneId, Entry>();
  /** How much larger than the configured size the window's text is, in CSS pixels. */
  #zoomPx = 0;
  /** A pass over the layout is wanted once the one in progress is done. */
  #scheduled = false;
  /** Which layout the views were last brought in line with. */
  #epoch: number | undefined;
  /** Out of sight, each of these panes lets go of its screen when its timer runs out. */
  readonly #releases = new Map<PaneId, number>();

  constructor(services: ViewsServices) {
    this.#services = services;
    services.layout.subscribe(() => this.#schedule());
    services.area.onBox(() => {
      this.#plan();
      this.#schedule();
    });
  }

  /** The view of a pane, if the pane is a terminal of this window. */
  get(pane: PaneId): TerminalView | undefined {
    return this.#entries.get(pane)?.view;
  }

  /** The panes that have a view. */
  panes(): ReadonlySet<PaneId> {
    return new Set(this.#entries.keys());
  }

  /** The pane that has the keyboard, and its view. */
  focused(): { readonly pane: PaneId; readonly view: TerminalView } | undefined {
    const pane = selectFocus(this.#services.layout.getState());
    const view = pane === null ? undefined : this.get(pane);
    return pane !== null && view ? { pane, view } : undefined;
  }

  /** The pane that shows a session, and its view. */
  bySession(session: string): { readonly pane: PaneId; readonly view: TerminalView } | undefined {
    for (const [pane, entry] of this.#entries) {
      if (entry.view.session === session) return { pane, view: entry.view };
    }
    return undefined;
  }

  /**
   * Resolves once the view of a pane is no longer in the middle of attaching to its
   * session. The core counts a session as shown from the moment it has attached this
   * page to it, which is a moment before the page has heard so.
   */
  async settled(pane: PaneId): Promise<void> {
    for (;;) {
      const attaching = this.#entries.get(pane)?.attaching;
      if (!attaching) return;
      await attaching;
    }
  }

  // The elements.

  /** Hands a pane's view the elements of its card. */
  mount(pane: PaneId, elements: ViewElements): void {
    // A card can be rendered a moment before the views have followed the layout.
    this.reconcile();
    const view = this.get(pane);
    if (!view) return;
    this.#keep(pane);
    view.mount(elements);
    if (selectFocus(this.#services.layout.getState()) === pane) view.focus();
  }

  /**
   * Takes the elements of a pane's card back. The view and its session carry on; when
   * the configuration says so, the view lets go of its screen after a while unseen.
   */
  unmount(pane: PaneId): void {
    const view = this.get(pane);
    if (!view) return;
    view.unmount();
    this.#armRelease(pane);
  }

  // Panes out of sight.

  /** How long a pane stays out of sight before it lets go of its screen; none: it keeps it. */
  #hiddenReleaseMs(): number | undefined {
    const terminal = this.#services.config()?.config.terminal;
    if (terminal?.["hidden-release"] !== true) return undefined;
    return (terminal["hidden-release-seconds"] ?? HIDDEN_RELEASE_DEFAULT_SECONDS) * 1000;
  }

  /** Lets a pane out of sight let go of its screen once the time the configuration gives is up. */
  #armRelease(pane: PaneId): void {
    const ms = this.#hiddenReleaseMs();
    if (ms === undefined || this.#releases.has(pane)) return;
    const view = this.get(pane);
    if (!view || view.mounted || view.released) return;
    const timer = window.setTimeout(() => {
      this.#releases.delete(pane);
      const now = this.get(pane);
      // Shown again, gone, or the configuration changed its mind meanwhile.
      if (now !== view || view.mounted || this.#hiddenReleaseMs() === undefined) return;
      view.release();
    }, ms);
    this.#releases.set(pane, timer);
  }

  /** A pane is on show, or gone: it keeps its screen. */
  #keep(pane: PaneId): void {
    const timer = this.#releases.get(pane);
    if (timer !== undefined) window.clearTimeout(timer);
    this.#releases.delete(pane);
  }

  /** Gives the keyboard to the pane the layout says has it, if its card is rendered. */
  focus(): void {
    this.focused()?.view.focus();
  }

  // How the views are set up.

  #settings(pane: PaneId): ViewSettings {
    const terminal = this.#services.config()?.config.terminal;
    return {
      appearance: this.#services.appearanceOf(pane),
      cursor: {
        style: terminal?.["cursor-style"] ?? "block",
        blink: terminal?.["cursor-blink"] ?? true,
      },
      scrollbackLines: terminal?.["scrollback-lines"] ?? SCROLLBACK_DEFAULT_LINES,
      ...(terminal?.["history-paging"] === true
        ? {
            history: {
              windowLines: terminal["window-history-lines"] ?? WINDOW_HISTORY_DEFAULT_LINES,
              releaseMs:
                (terminal["history-release-seconds"] ?? HISTORY_RELEASE_DEFAULT_SECONDS) * 1000,
            },
          }
        : {}),
      zoomPx: this.#zoomPx,
    };
  }

  /** Sets every view up again: the configuration or the zoom changed. */
  configure(): void {
    const releasing = this.#hiddenReleaseMs() !== undefined;
    for (const [pane, entry] of this.#entries) {
      const settings = this.#settings(pane);
      entry.look = settings.appearance;
      entry.view.configure(settings);
      // Panes out of sight start counting when the setting is turned on, and stop when off.
      if (releasing) this.#armRelease(pane);
      else this.#keep(pane);
    }
    this.#plan();
  }

  /**
   * Gives each view its look again where it is no longer the one it has: the profiles,
   * or what a pane was started from, changed.
   */
  restyle(): void {
    let changed = false;
    for (const [pane, entry] of this.#entries) changed = this.#restyle(pane, entry) || changed;
    if (changed) this.#plan();
  }

  #restyle(pane: PaneId, entry: Entry): boolean {
    const look = this.#services.appearanceOf(pane);
    if (look === entry.look) return false;
    entry.look = look;
    entry.view.configure(this.#settings(pane));
    return true;
  }

  /** How much larger than the configured size the window's text is, in CSS pixels. */
  get zoomPx(): number {
    return this.#zoomPx;
  }

  /**
   * Makes the window's text larger or smaller than configured by `px`, within what a
   * font size may be. Nought is the configured size.
   */
  setZoom(px: number): void {
    const base = this.#services.appearance().fontSizePx;
    const size = Math.min(Math.max(base + px, FONT_SIZE_MIN_PX), FONT_SIZE_MAX_PX);
    if (size - base === this.#zoomPx) return;
    this.#zoomPx = size - base;
    this.configure();
  }

  // Following the layout.

  #schedule(): void {
    if (this.#scheduled) return;
    this.#scheduled = true;
    // Several changes arrive in one notice; they are followed together.
    queueMicrotask(() => {
      this.#scheduled = false;
      this.reconcile();
    });
  }

  /** Brings the views in line with the layout as it is now. */
  reconcile(): void {
    const { layout, titles, reports } = this.#services;
    const state = layout.getState();
    if (!state.ready) return;
    // Another layout altogether: nothing of the old one's terminals carries over.
    if (this.#epoch !== undefined && this.#epoch !== state.epoch) {
      this.dispose();
      titles.clear();
      reports.clear();
    }
    this.#epoch = state.epoch;
    const panes = new Set(selectWindowPanes(state));
    // The scratch terminal, put away, stays with the window that had it on show.
    const parked = state.scratch?.parked?.pane;
    if (parked && this.#entries.has(parked.id)) panes.add(parked.id);
    const contentOf = (pane: PaneId): PaneContent | undefined =>
      pane === parked?.id ? parked.content : state.panes[pane]?.content;
    for (const [pane, entry] of this.#entries) {
      if (panes.has(pane)) continue;
      this.#keep(pane);
      entry.view.dispose();
      this.#entries.delete(pane);
      titles.forget(pane);
      reports.forget(pane);
    }

    const sessionless: PaneId[] = [];
    for (const pane of panes) {
      const content = contentOf(pane);
      if (content?.type !== "terminal") continue;
      const entry = this.#entries.get(pane) ?? this.#create(pane);
      if (content.session === null) {
        if (!entry.asked) sessionless.push(pane);
      } else if (entry.session !== content.session) {
        entry.session = content.session;
        entry.asked = false;
        const attaching = this.#attach(pane, entry, content.backend, content.session).then(() => {
          if (entry.attaching === attaching) entry.attaching = undefined;
        });
        entry.attaching = attaching;
      }
    }
    // A session is started at the grid its pane has, which is not known before the
    // window's contents have been laid out.
    if (sessionless.length > 0 && this.#services.area.box()) this.#start(sessionless);
  }

  #create(pane: PaneId): Entry {
    const { shared, titles, reports } = this.#services;
    // A pane's title is the one its session reports, so a page that attaches later is
    // told it too.
    const retitle = () => titles.set(pane, reports.get(pane).title ?? "");
    const view = new TerminalView(
      {
        ...shared,
        shortcut: (event, context) => this.#services.shortcut(pane, event, context.alternateScreen),
        attached: (about) => {
          reports.attached(pane, about);
          retitle();
          // What the session was started from decides its look.
          const held = this.#entries.get(pane);
          if (about && held) this.#restyle(pane, held);
        },
        reported: (event) => {
          reports.reported(pane, event);
          if (event.type === "title" || event.type === "state") retitle();
        },
        exited: (status) => reports.exited(pane, status),
        sizedElsewhere: (sizedElsewhere) => reports.sizedElsewhere(pane, sizedElsewhere),
        focused: () => this.#tookFocus(pane),
        restart: () => this.restart(pane),
        searched: () => this.#services.searched(pane),
      },
      this.#settings(pane),
    );
    const entry: Entry = {
      view,
      session: null,
      asked: false,
      attaching: undefined,
      look: this.#services.appearanceOf(pane),
    };
    this.#entries.set(pane, entry);
    const planned = this.#plannedSize(this.#services.layout.getState(), pane);
    if (planned) view.setPlannedSize(planned);
    return entry;
  }

  async #attach(
    pane: PaneId,
    entry: Entry,
    backend: string | null,
    session: string,
  ): Promise<void> {
    const { client } = this.#services;
    try {
      await entry.view.attach(
        client.transport,
        backend ?? client.transport.defaultBackend,
        session,
      );
      if (this.#entries.get(pane) !== entry) entry.view.dispose();
      // A pane that has never been on show counts as out of sight from now.
      else this.#armRelease(pane);
    } catch (error) {
      // The pane has moved on to another session, or gone, while this was tried.
      if (this.#entries.get(pane) !== entry || entry.session !== session) return;
      if (error instanceof CommandError && GONE.has(error.code)) {
        entry.view.showLost("The program in this pane has ended.");
      } else {
        entry.view.showMessage(`The pane's session cannot be shown: ${reason(error)}`);
      }
    }
  }

  /** Has the core start sessions for panes that have none. */
  #start(panes: readonly PaneId[]): void {
    const state = this.#services.layout.getState();
    const asked = panes.flatMap((pane) => {
      const entry = this.#entries.get(pane);
      if (!entry) return [];
      entry.asked = true;
      return [{ pane, size: this.#sizeOf(state, pane, entry.view) }];
    });
    if (asked.length === 0) return;
    this.#services.client.sessionsStart(asked).then(
      (outcome) => {
        // A session that was started arrives as a change to its pane.
        for (const failed of outcome.failed) {
          this.#entries
            .get(failed.pane)
            ?.view.showLost(`A session could not be started: ${failed.message}`);
        }
      },
      (error: unknown) => {
        for (const { pane } of asked) {
          this.#entries
            .get(pane)
            ?.view.showLost(`A session could not be started: ${reason(error)}`);
        }
      },
    );
  }

  /** Starts a new session in a pane whose program has ended. */
  restart(pane: PaneId): void {
    const entry = this.#entries.get(pane);
    if (!entry) return;
    this.#start([pane]);
  }

  #tookFocus(pane: PaneId): void {
    const { layout, client } = this.#services;
    this.#services.keyboard(pane);
    if (selectFocus(layout.getState()) === pane) return;
    client.layoutApply({ type: "focus", target: { type: "pane", pane } }).catch(() => undefined);
  }

  // The grid a pane has, or will have.

  #sizeOf(state: LayoutState, pane: PaneId, view: TerminalView): TermSize {
    return (view.mounted ? undefined : this.#plannedSize(state, pane)) ?? view.gridSize();
  }

  /** Tells every view without elements what grid its pane's place in its tab gives it. */
  #plan(): void {
    const state = this.#services.layout.getState();
    for (const [pane, entry] of this.#entries) {
      if (entry.view.mounted) continue;
      const planned = this.#plannedSize(state, pane);
      if (planned) entry.view.setPlannedSize(planned);
    }
  }

  /**
   * The grid a pane has by its place in its tab, worked out the way its card and its
   * renderer would arrive at it, for a pane that has neither.
   */
  #plannedSize(state: LayoutState, pane: PaneId): TermSize | undefined {
    const { area, shared } = this.#services;
    const box = area.box();
    if (!box) return undefined;
    const tab = state.tabs[state.paneTabs[pane] ?? ""];
    const float = state.floats[pane];
    let rect;
    let beside = 1;
    if (tab) {
      beside = panesOf(tab.root).length;
      const placed = computeRects(shapeOf(state, tab.root), box, area.gapPx()).panes;
      rect = placed.find((candidate) => candidate.pane === pane)?.rect;
    } else if (float) {
      rect = unitRectToPx(float.rect, box);
    }
    if (!rect) return undefined;
    const settings = this.#settings(pane);
    const fontSizePx = Math.min(
      Math.max(settings.appearance.fontSizePx + settings.zoomPx, FONT_SIZE_MIN_PX),
      FONT_SIZE_MAX_PX,
    );
    const dpr = window.devicePixelRatio;
    const metrics = shared.host.measureCells(rendererTheme(settings.appearance, fontSizePx), dpr);
    return gridFor(rect, area.chrome(beside, float !== undefined), metrics, dpr);
  }

  /** Disposes of every view. Their sessions carry on. */
  dispose(): void {
    for (const pane of [...this.#releases.keys()]) this.#keep(pane);
    for (const entry of this.#entries.values()) entry.view.dispose();
    this.#entries.clear();
  }
}
