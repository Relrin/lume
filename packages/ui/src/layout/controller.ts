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
 * core then says the ratios are is what holds.
 */

import type { Tokens } from "@lume/tokens";

import type { ConfigView } from "../core/types.ts";
import { DIVIDER_INTENT_INTERVAL_MS, DIVIDER_SETTLE_TIMEOUT_MS, PANE_MIN_PX } from "../tuning.ts";
import { shapeOf, type CardChrome } from "./estimate.ts";
import {
  computeRects,
  dragRatios,
  equalRatios,
  type Divider,
  type Rect,
  type Rects,
} from "./geometry.ts";
import { panesOf, selectActiveTab } from "./selectors.ts";
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
   * itself.
   */
  chrome(panes: number): CardChrome {
    const tokens = this.#tokens();
    const mode = this.#services.config()?.config.appearance["pane-header"] ?? "always";
    const header = mode === "always" || (mode === "auto" && panes > 1);
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
    this.layout();
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

  /** How many panes the tab that holds a pane has: what decides whether it has a header. */
  panesBeside(pane: PaneId): number {
    const state = this.#services.layout.getState();
    const tab = state.tabs[state.paneTabs[pane] ?? ""];
    return tab ? panesOf(tab.root).length : 1;
  }
}
