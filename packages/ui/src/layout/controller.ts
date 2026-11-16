/**
 * Puts the panes of the tab on show where the layout says they go.
 *
 * The component tree decides which cards and dividers exist. Where each lies is decided
 * here and written straight to the elements, so that dragging a divider or resizing the
 * window moves pixels without rendering anything again: a terminal that is being
 * resized sixty times a second must not wait for the component tree.
 *
 * A divider being dragged follows the pointer at once, with ratios of its own. The core
 * is told a few times a second and once more when the pointer lets go, and what the
 * core then says the ratios are is what holds. A floating pane being moved or resized
 * is followed the same way, with a rectangle of its own.
 */

import type { Tokens } from "@lume/tokens";

import type { ConfigView } from "../core/types.ts";
import {
  DIVIDER_INTENT_INTERVAL_MS,
  DIVIDER_SETTLE_TIMEOUT_MS,
  DRAG_SETTLE_TIMEOUT_MS,
  FLOAT_INTENT_INTERVAL_MS,
  FLOAT_MIN_PX,
  PANE_MIN_PX,
} from "../tuning.ts";
import { shapeOf, type CardChrome } from "./estimate.ts";
import { dragFloat, type FloatGrip } from "./floats.ts";
import {
  computeRects,
  dragRatios,
  equalRatios,
  pxToUnitRect,
  unitRectToPx,
  type Divider,
  type Rect,
  type Rects,
} from "./geometry.ts";
import { panesOf, selectActiveTab, selectFloating } from "./selectors.ts";
import type { LayoutState, LayoutStore } from "./store.ts";
import type { Intent, PaneId, SplitId, TabId } from "./types.ts";

/** What the controller needs from its surroundings. */
export interface AreaServices {
  readonly layout: LayoutStore;
  /** The configuration in use, or nothing before the core has sent one. */
  config(): ConfigView | null;
  /** Asks the core to change the layout. */
  apply(intent: Intent): Promise<unknown>;
  /** The tokens a page starts with, for the time before the core has sent any. */
  readonly defaults: Tokens;
}

/** Names a divider: the split it belongs to and its place among the split's gaps. */
export function dividerKey(split: SplitId, index: number): string {
  return `${split}:${index}`;
}

/** Ratios that are being tried out for a split, in place of the layout's. */
interface Override {
  readonly split: SplitId;
  ratios: readonly number[];
}

/** A divider that the pointer has hold of. */
interface Drag {
  readonly pointerId: number;
  readonly divider: Divider;
  /** Where the pointer was along the split's direction when it took hold. */
  readonly start: number;
  /** The split's ratios at that moment. */
  readonly ratios: readonly number[];
  /** When the core was last told, on the page's clock. */
  told: number;
  /** The ratios the core was last told. */
  sent: readonly number[] | undefined;
}

/** A rectangle that is being tried out for a floating pane, in place of the layout's. */
interface FloatOverride {
  readonly pane: PaneId;
  readonly rect: Rect;
  /**
   * The layout's revision once the core had taken the last rectangle it was told.
   * Nothing until the core has answered.
   */
  settledAt: number | undefined;
}

/** A floating pane that the pointer has hold of. */
interface FloatDrag {
  readonly pointerId: number;
  readonly pane: PaneId;
  readonly grip: FloatGrip;
  /** Where the pointer was when it took hold. */
  readonly x: number;
  readonly y: number;
  /** The pane's rectangle at that moment. */
  readonly rect: Rect;
  /** When the core was last told, on the page's clock. */
  told: number;
  /** What the core was last told, written out. */
  sent: string | undefined;
  /** Stops listening to the pointer. */
  release(): void;
}

/** The first card that floats is drawn above everything that does not. */
const FLOAT_Z_BASE = 20;

/** The revision an answer to an applied intent names, if it is such an answer. */
function revisionOf(applied: unknown): number | undefined {
  if (typeof applied !== "object" || applied === null || !("revision" in applied)) return undefined;
  return typeof applied.revision === "number" ? applied.revision : undefined;
}

function sameRatios(a: readonly number[], b: readonly number[] | undefined): boolean {
  return b !== undefined && a.length === b.length && a.every((ratio, index) => ratio === b[index]);
}

/** Lays out the tab on show inside one element. */
export class TabAreaController {
  readonly #services: AreaServices;
  readonly #panes = new Map<PaneId, HTMLElement>();
  readonly #dividers = new Map<string, HTMLElement>();
  /** What was last written to each element, so nothing is written twice. */
  readonly #written = new WeakMap<HTMLElement, string>();
  readonly #boxListeners = new Set<() => void>();

  #container: HTMLElement | undefined;
  #size: { width: number; height: number } | undefined;
  #rects: Rects = { panes: [], dividers: [] };
  #override: Override | undefined;
  #drag: Drag | undefined;
  #settle: number | undefined;
  #floatRects = new Map<PaneId, Rect>();
  #floatOverride: FloatOverride | undefined;
  #floatDrag: FloatDrag | undefined;
  #floatSettle: number | undefined;
  /** The core's answer to the last place of a floating pane it was told. */
  #lastFloatTold: Promise<unknown> = Promise.resolve(undefined);

  constructor(services: AreaServices) {
    this.#services = services;
    services.layout.subscribe(() => this.#onLayout());
  }

  // What the panes are laid out with.

  #tokens(): Tokens {
    return this.#services.config()?.tokens ?? this.#services.defaults;
  }

  /** The gap between neighbouring cards, and between the cards and the window's edge. */
  gapPx(): number {
    return Math.max(0, Math.round(this.#tokens()["space.gap"]));
  }

  /**
   * The parts of a card that are not terminal, for a card in a tab of `panes` panes. A
   * header that is shown only when it is needed is left off a pane that has its tab to
   * itself. A floating card always has its header: it is what the card is moved by.
   */
  chrome(panes: number, floating = false): CardChrome {
    const tokens = this.#tokens();
    const mode = this.#services.config()?.config.appearance["pane-header"] ?? "always";
    const header = floating || mode === "always" || (mode === "auto" && panes > 1);
    return {
      headerPx: header ? Math.round(tokens["size.header"]) : 0,
      borderPx: Math.round(tokens["size.border"]),
    };
  }

  /**
   * The space the tabs' panes are laid out in, in the container's own coordinates, or
   * nothing while the container has no size.
   */
  box(): Rect | undefined {
    const size = this.#size;
    if (!size) return undefined;
    const gap = this.gapPx();
    // The bar above the area keeps its own distance, so there is no gap at the top.
    return {
      x: gap,
      y: 0,
      width: Math.max(0, size.width - 2 * gap),
      height: Math.max(0, size.height - gap),
    };
  }

  /** Calls `listener` when the space the panes are laid out in changes. */
  onBox(listener: () => void): () => void {
    this.#boxListeners.add(listener);
    return () => this.#boxListeners.delete(listener);
  }

  /** The tab on show laid out as it is now, a drag in progress included. */
  rects(): Rects {
    return this.#rects;
  }

  /** Where a floating pane of this window is drawn, a drag in progress included. */
  floatRect(pane: PaneId): Rect | undefined {
    return this.#floatRects.get(pane);
  }

  /** The element the panes are laid out in, while there is one. */
  container(): HTMLElement | undefined {
    return this.#container;
  }

  /** The card of a pane, while it is rendered. */
  paneElement(pane: PaneId): HTMLElement | undefined {
    return this.#panes.get(pane);
  }

  // The elements.

  /** Starts laying out inside `container`. Returns how to stop. */
  attach(container: HTMLElement): () => void {
    this.#container = container;
    const measure = () => {
      const size = { width: container.clientWidth, height: container.clientHeight };
      if (size.width === 0 || size.height === 0) return;
      const changed = this.#size?.width !== size.width || this.#size.height !== size.height;
      this.#size = size;
      this.layout();
      if (changed) for (const listener of [...this.#boxListeners]) listener();
    };
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    measure();
    return () => {
      observer.disconnect();
      if (this.#container === container) this.#container = undefined;
    };
  }

  /** Gives the controller the card of a pane to place. Returns how to take it back. */
  registerPane(pane: PaneId, element: HTMLElement): () => void {
    this.#panes.set(pane, element);
    this.layout();
    return () => {
      if (this.#panes.get(pane) === element) this.#panes.delete(pane);
    };
  }

  /** Gives the controller a divider to place and to drag. Returns how to take it back. */
  registerDivider(split: SplitId, index: number, element: HTMLElement): () => void {
    const key = dividerKey(split, index);
    this.#dividers.set(key, element);
    const down = (event: PointerEvent) => this.#onPointerDown(split, index, element, event);
    const move = (event: PointerEvent) => this.#onPointerMove(event);
    const up = (event: PointerEvent) => this.#onPointerUp(event);
    const equalise = () => this.equalise(split);
    element.addEventListener("pointerdown", down);
    element.addEventListener("pointermove", move);
    element.addEventListener("pointerup", up);
    element.addEventListener("pointercancel", up);
    element.addEventListener("dblclick", equalise);
    this.layout();
    return () => {
      element.removeEventListener("pointerdown", down);
      element.removeEventListener("pointermove", move);
      element.removeEventListener("pointerup", up);
      element.removeEventListener("pointercancel", up);
      element.removeEventListener("dblclick", equalise);
      if (this.#dividers.get(key) === element) this.#dividers.delete(key);
    };
  }

  // Laying out.

  #tab(state: LayoutState): TabId | null {
    return state.ready ? selectActiveTab(state) : null;
  }

  /** Works out where everything goes and writes it to the elements that changed. */
  layout(): void {
    const state = this.#services.layout.getState();
    const box = this.box();
    this.#layoutFloats(state, box);
    const tab = state.tabs[this.#tab(state) ?? ""];
    if (!box || !tab) {
      this.#rects = { panes: [], dividers: [] };
      return;
    }
    const override = this.#override;
    this.#rects = computeRects(shapeOf(state, tab.root), box, this.gapPx(), {
      zoomed: tab.zoomed,
      ratios: (split) => (override?.split === split ? override.ratios : undefined),
    });
    for (const placed of this.#rects.panes) {
      const element = this.#panes.get(placed.pane);
      if (element) this.#place(element, placed.rect);
    }
    for (const divider of this.#rects.dividers) {
      const element = this.#dividers.get(dividerKey(divider.split, divider.index));
      if (element) this.#place(element, divider.rect);
    }
  }

  /**
   * Places the panes that float over this window: each where the layout says, or where
   * the pointer has it, and each above the ones behind it.
   */
  #layoutFloats(state: LayoutState, box: Rect | undefined): void {
    const rects = new Map<PaneId, Rect>();
    if (box && state.ready) {
      const override = this.#floatOverride;
      selectFloating(state).forEach((pane, order) => {
        const float = state.floats[pane];
        // A pane that is put away has no card.
        if (!float || float.minimised !== null) return;
        const rect = override?.pane === pane ? override.rect : unitRectToPx(float.rect, box);
        rects.set(pane, rect);
        const element = this.#panes.get(pane);
        if (!element) return;
        this.#place(element, rect);
        const above = String(FLOAT_Z_BASE + order);
        if (element.style.zIndex !== above) element.style.zIndex = above;
      });
    }
    this.#floatRects = rects;
  }

  /**
   * Writes a rectangle to an element through its style object, which a policy that
   * forbids inline style attributes still allows.
   */
  #place(element: HTMLElement, rect: Rect): void {
    const written = `${rect.x},${rect.y},${rect.width},${rect.height}`;
    if (this.#written.get(element) === written) return;
    this.#written.set(element, written);
    element.style.transform = `translate(${rect.x}px, ${rect.y}px)`;
    element.style.width = `${rect.width}px`;
    element.style.height = `${rect.height}px`;
  }

  #onLayout(): void {
    // Ratios that were being tried out give way once the layout has caught up with
    // them, or no longer has the split.
    const override = this.#override;
    if (override && !this.#drag) {
      const held = this.#services.layout.getState().splits[override.split]?.ratios;
      if (held === undefined || sameRatios(override.ratios, held)) this.#release();
    }
    // A floating pane's own rectangle gives way once the layout holds what the core
    // made of the last one it was told, or the pane floats no more.
    const float = this.#floatOverride;
    if (float && !this.#floatDrag) {
      const state = this.#services.layout.getState();
      const caughtUp = float.settledAt !== undefined && state.revision >= float.settledAt;
      if (caughtUp || !state.floats[float.pane]) this.#releaseFloat();
    }
    this.layout();
  }

  #releaseFloat(): void {
    this.#floatOverride = undefined;
    if (this.#floatSettle !== undefined) window.clearTimeout(this.#floatSettle);
    this.#floatSettle = undefined;
  }

  #release(): void {
    this.#override = undefined;
    if (this.#settle !== undefined) window.clearTimeout(this.#settle);
    this.#settle = undefined;
  }

  // Dragging a divider.

  #along(divider: Divider, event: PointerEvent): number {
    return divider.axis === "row" ? event.clientX : event.clientY;
  }

  #onPointerDown(split: SplitId, index: number, element: HTMLElement, event: PointerEvent): void {
    if (event.button !== 0 || this.#drag) return;
    const divider = this.#rects.dividers.find(
      (candidate) => candidate.split === split && candidate.index === index,
    );
    const ratios = this.#services.layout.getState().splits[split]?.ratios;
    if (!divider || !ratios) return;
    event.preventDefault();
    element.setPointerCapture(event.pointerId);
    element.dataset["dragging"] = "true";
    // A divider taken hold of again before the layout caught up starts where it was left.
    const starting = this.#override?.split === split ? this.#override.ratios : ratios;
    this.#release();
    this.#drag = {
      pointerId: event.pointerId,
      divider,
      start: this.#along(divider, event),
      ratios: starting,
      told: 0,
      sent: undefined,
    };
    this.#override = { split, ratios: starting };
  }

  #onPointerMove(event: PointerEvent): void {
    const drag = this.#drag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const { divider } = drag;
    const min = divider.axis === "row" ? PANE_MIN_PX.width : PANE_MIN_PX.height;
    const ratios = dragRatios(
      drag.ratios,
      divider.index,
      this.#along(divider, event) - drag.start,
      divider.extentPx,
      this.gapPx(),
      min,
    );
    this.#override = { split: divider.split, ratios };
    this.layout();
    const now = performance.now();
    if (now - drag.told >= DIVIDER_INTENT_INTERVAL_MS) this.#tell(drag, ratios, now);
  }

  #onPointerUp(event: PointerEvent): void {
    const drag = this.#drag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    this.#drag = undefined;
    const element = this.#dividers.get(dividerKey(drag.divider.split, drag.divider.index));
    if (element) delete element.dataset["dragging"];
    const override = this.#override;
    if (!override) return;
    // The last position is always told, however lately the one before it was.
    const settled = this.#tell(drag, override.ratios, performance.now());
    // The ratios stay as the pointer left them until the layout says the same, so the
    // divider does not jump back for the moment the answer takes.
    void settled.then(
      () => this.#onLayout(),
      () => {
        this.#release();
        this.layout();
      },
    );
    this.#settle = window.setTimeout(() => {
      this.#release();
      this.layout();
    }, DIVIDER_SETTLE_TIMEOUT_MS);
  }

  #tell(drag: Drag, ratios: readonly number[], now: number): Promise<unknown> {
    drag.told = now;
    if (sameRatios(ratios, drag.sent)) return Promise.resolve();
    drag.sent = ratios;
    const sent = this.#services.apply({
      type: "resize",
      split: drag.divider.split,
      ratios: [...ratios],
    });
    // A refusal while dragging is passed over: the next position is told anyway.
    sent.catch(() => undefined);
    return sent;
  }

  /** Gives every child of a split the same share. */
  equalise(split: SplitId): void {
    const count = this.#services.layout.getState().splits[split]?.ratios.length;
    if (count === undefined) return;
    this.#services
      .apply({ type: "resize", split, ratios: equalRatios(count) })
      .catch(() => undefined);
  }

  // Moving and resizing a floating pane.

  /**
   * Lets the pointer that was pressed on `handle` move a floating pane, or one of its
   * edges, until it lets go. `handle` is the element the press was on: the header of
   * the pane's card, or one of the card's edge handles.
   */
  startFloatDrag(pane: PaneId, grip: FloatGrip, event: PointerEvent, handle: HTMLElement): void {
    if (event.button !== 0 || this.#floatDrag) return;
    const rect = this.#floatRects.get(pane);
    if (!rect) return;
    const move = (moved: PointerEvent) => this.#onFloatMove(moved);
    const up = (ended: PointerEvent) => this.#onFloatUp(ended);
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
    handle.addEventListener("pointercancel", up);
    handle.setPointerCapture(event.pointerId);
    const card = this.#panes.get(pane);
    if (card) card.dataset["moving"] = "true";
    this.#releaseFloat();
    this.#floatDrag = {
      pointerId: event.pointerId,
      pane,
      grip,
      x: event.clientX,
      y: event.clientY,
      rect,
      told: performance.now(),
      sent: undefined,
      release: () => {
        handle.removeEventListener("pointermove", move);
        handle.removeEventListener("pointerup", up);
        handle.removeEventListener("pointercancel", up);
        if (card) delete card.dataset["moving"];
      },
    };
    this.#floatOverride = { pane, rect, settledAt: undefined };
  }

  #onFloatMove(event: PointerEvent): void {
    const drag = this.#floatDrag;
    const box = this.box();
    if (!drag || drag.pointerId !== event.pointerId || !box) return;
    const rect = dragFloat(
      drag.rect,
      drag.grip,
      event.clientX - drag.x,
      event.clientY - drag.y,
      box,
      FLOAT_MIN_PX,
    );
    this.#floatOverride = { pane: drag.pane, rect, settledAt: undefined };
    this.layout();
    const now = performance.now();
    if (now - drag.told >= FLOAT_INTENT_INTERVAL_MS) this.#tellFloat(drag, rect, box, now);
  }

  #onFloatUp(event: PointerEvent): void {
    const drag = this.#floatDrag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    this.#floatDrag = undefined;
    drag.release();
    const override = this.#floatOverride;
    const box = this.box();
    // A press that moved nothing leaves the pane where the layout has it.
    if (!override || !box || override.rect === drag.rect) {
      this.#releaseFloat();
      this.layout();
      return;
    }
    // The last place is always told. The card stays where the pointer left it until
    // the layout holds what the core made of that place.
    const settled = this.#tellFloat(drag, override.rect, box, performance.now());
    void settled.then(
      (applied) => {
        if (this.#floatOverride !== override) return;
        override.settledAt = revisionOf(applied) ?? 0;
        this.#onLayout();
      },
      () => {
        if (this.#floatOverride !== override) return;
        this.#releaseFloat();
        this.layout();
      },
    );
    this.#floatSettle = window.setTimeout(() => {
      this.#releaseFloat();
      this.layout();
    }, DRAG_SETTLE_TIMEOUT_MS);
  }

  #tellFloat(drag: FloatDrag, rect: Rect, box: Rect, now: number): Promise<unknown> {
    drag.told = now;
    const unit = pxToUnitRect(rect, box);
    const written = `${unit.x},${unit.y},${unit.w},${unit.h}`;
    if (written === drag.sent) return this.#lastFloatTold;
    drag.sent = written;
    const sent = this.#services.apply({ type: "float_move", pane: drag.pane, rect: unit });
    // A refusal while dragging is passed over: the next place is told anyway.
    sent.catch(() => undefined);
    this.#lastFloatTold = sent;
    return sent;
  }

  /** How many panes the tab that holds a pane has: what decides whether it has a header. */
  panesBeside(pane: PaneId): number {
    const state = this.#services.layout.getState();
    const tab = state.tabs[state.paneTabs[pane] ?? ""];
    return tab ? panesOf(tab.root).length : 1;
  }
}
