/**
 * Dragging a pane's card by its header to another place in its window.
 *
 * A card that is dragged stays where it is, dimmed, while a preview shows where it
 * would go: beside another card, in another card's place, or into a tab of its own
 * when it is held over the window's tabs. Letting go asks the core to move it. The
 * preview is one element moved through its style object, so a drag renders nothing.
 */

import type { TabAreaController } from "../layout/controller.ts";
import type { Rect } from "../layout/geometry.ts";
import { panesOf } from "../layout/selectors.ts";
import type { LayoutStore } from "../layout/store.ts";
import type { Intent, PaneId } from "../layout/types.ts";
import { DRAG_START_PX } from "../tuning.ts";
import { dropTarget, type DropTarget } from "./targets.ts";

/** What dragging cards needs from its surroundings. */
export interface CardDragServices {
  readonly layout: LayoutStore;
  readonly area: TabAreaController;
  /** Asks the core to change the layout. */
  apply(intent: Intent): Promise<unknown>;
  /** Tells the user that the card could not be put where it was dropped. */
  failed(message: string): void;
}

/** The attribute that marks an element a card may be dropped on to get a tab of its own. */
export const NEW_TAB_DROP = "new-tab";

/** Where the card in hand would go: among the cards, or into a tab of its own. */
type Target = DropTarget | { readonly kind: "new-tab"; readonly element: HTMLElement };

/** A card that the pointer has hold of. */
interface Held {
  readonly pointerId: number;
  readonly pane: PaneId;
  readonly x: number;
  readonly y: number;
  /** Whether the pointer has moved far enough for this to be a drag and not a click. */
  dragging: boolean;
  target: Target | undefined;
  /** Stops listening to the pointer and the keyboard. */
  release(): void;
}

/** What the preview says when letting go would exchange two cards. */
const SWAP_LABEL = "Swap places";

function sameTarget(a: Target | undefined, b: Target | undefined): boolean {
  if (a === undefined || b === undefined) return a === b;
  if (a.kind === "new-tab") return b.kind === "new-tab" && a.element === b.element;
  if (b.kind === "new-tab") return false;
  if (a.kind !== b.kind || a.pane !== b.pane) return false;
  return a.kind !== "beside" || (b.kind === "beside" && a.edge === b.edge);
}

/** Moves cards about their window by their headers. */
export class CardDrag {
  readonly #services: CardDragServices;
  #held: Held | undefined;
  #preview: HTMLElement | undefined;

  constructor(services: CardDragServices) {
    this.#services = services;
  }

  /** Whether a card is being dragged now. */
  get active(): boolean {
    return this.#held?.dragging === true;
  }

  /**
   * Lets the pointer that was pressed on a card's header drag the card until it lets
   * go. A press that does not move is not a drag, and changes nothing.
   */
  start(pane: PaneId, event: PointerEvent, header: HTMLElement): void {
    if (event.button !== 0 || this.#held) return;
    const move = (moved: PointerEvent) => this.#onMove(moved);
    const up = (ended: PointerEvent) => this.#onUp(ended, ended.type === "pointerup");
    const key = (pressed: KeyboardEvent) => {
      if (pressed.key !== "Escape" || !this.#held?.dragging) return;
      pressed.preventDefault();
      pressed.stopPropagation();
      this.#end();
    };
    header.addEventListener("pointermove", move);
    header.addEventListener("pointerup", up);
    header.addEventListener("pointercancel", up);
    window.addEventListener("keydown", key, true);
    header.setPointerCapture(event.pointerId);
    this.#held = {
      pointerId: event.pointerId,
      pane,
      x: event.clientX,
      y: event.clientY,
      dragging: false,
      target: undefined,
      release: () => {
        header.removeEventListener("pointermove", move);
        header.removeEventListener("pointerup", up);
        header.removeEventListener("pointercancel", up);
        window.removeEventListener("keydown", key, true);
        if (header.hasPointerCapture(event.pointerId)) {
          header.releasePointerCapture(event.pointerId);
        }
      },
    };
  }

  #onMove(event: PointerEvent): void {
    const held = this.#held;
    if (!held || held.pointerId !== event.pointerId) return;
    if (!held.dragging) {
      const far = Math.hypot(event.clientX - held.x, event.clientY - held.y);
      if (far < DRAG_START_PX) return;
      held.dragging = true;
      const card = this.#services.area.paneElement(held.pane);
      if (card) card.dataset["dragging"] = "true";
    }
    const target = this.#targetAt(held.pane, event.clientX, event.clientY);
    if (sameTarget(target, held.target)) return;
    this.#show(held.target, target);
    held.target = target;
  }

  #onUp(event: PointerEvent, dropped: boolean): void {
    const held = this.#held;
    if (!held || held.pointerId !== event.pointerId) return;
    const { pane, target, dragging } = held;
    this.#end();
    if (!dropped || !dragging || !target) return;
    const intent = this.#intentFor(pane, target);
    if (!intent) return;
    this.#services.apply(intent).catch((error: unknown) => {
      this.#services.failed(error instanceof Error ? error.message : String(error));
    });
  }

  /** Lets go of the card in hand without moving it. */
  #end(): void {
    const held = this.#held;
    if (!held) return;
    this.#held = undefined;
    held.release();
    this.#show(held.target, undefined);
    const card = this.#services.area.paneElement(held.pane);
    if (card) delete card.dataset["dragging"];
  }

  #intentFor(pane: PaneId, target: Target): Intent | undefined {
    switch (target.kind) {
      case "beside":
        return {
          type: "move",
          pane,
          to: { type: "beside", pane: target.pane, edge: target.edge },
        };
      case "swap":
        return { type: "swap", a: pane, b: target.pane };
      case "new-tab": {
        const window = this.#services.layout.getState().window;
        if (window === null) return undefined;
        return { type: "move", pane, to: { type: "new_tab", window } };
      }
    }
  }

  /** What the card would be dropped on at a point of the window. */
  #targetAt(pane: PaneId, clientX: number, clientY: number): Target | undefined {
    const { area, layout } = this.#services;
    const container = area.container();
    if (container) {
      const origin = container.getBoundingClientRect();
      const point = { x: clientX - origin.left, y: clientY - origin.top };
      const among = dropTarget(point, area.rects().panes, pane);
      if (among) return among;
    }
    // A pane that has its tab to itself gains nothing from a tab of its own.
    const state = layout.getState();
    const tab = state.tabs[state.paneTabs[pane] ?? ""];
    if (!tab || panesOf(tab.root).length < 2) return undefined;
    const under = document.elementFromPoint(clientX, clientY);
    const tabs = under?.closest<HTMLElement>(`[data-drop="${NEW_TAB_DROP}"]`);
    return tabs ? { kind: "new-tab", element: tabs } : undefined;
  }

  /** Takes the preview of one target away and shows that of another. */
  #show(before: Target | undefined, now: Target | undefined): void {
    if (before?.kind === "new-tab") delete before.element.dataset["dropActive"];
    if (now?.kind === "new-tab") now.element.dataset["dropActive"] = "true";
    if (!now || now.kind === "new-tab") {
      this.#preview?.remove();
      this.#preview = undefined;
      return;
    }
    const preview = this.#preview ?? this.#makePreview();
    if (!preview) return;
    this.#place(preview, now.rect);
    preview.dataset["kind"] = now.kind;
    // Set as text, like everything the page writes outside the component tree.
    preview.firstElementChild!.textContent = now.kind === "swap" ? SWAP_LABEL : "";
  }

  #makePreview(): HTMLElement | undefined {
    const container = this.#services.area.container();
    if (!container) return undefined;
    const preview = document.createElement("div");
    preview.className = "drop-preview";
    preview.setAttribute("aria-hidden", "true");
    const label = document.createElement("span");
    label.className = "drop-preview-label";
    preview.append(label);
    container.append(preview);
    this.#preview = preview;
    return preview;
  }

  #place(element: HTMLElement, rect: Rect): void {
    element.style.transform = `translate(${rect.x}px, ${rect.y}px)`;
    element.style.width = `${rect.width}px`;
    element.style.height = `${rect.height}px`;
  }
}
