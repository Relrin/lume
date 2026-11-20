/**
 * What the colour picker works with: a colour typed as `#rgb` or `#rrggbb`, the same
 * colour as hue, saturation and brightness for the square and the bar it is picked
 * on, and the one picker a window has open at a time.
 *
 * A picker is asked for with what to call the colour, the colour to start from and
 * what to do with the colour once it is applied. Nothing is done until it is: the
 * picker shows the colour as it is mixed, and Cancel leaves everything as it was.
 * Nothing here touches the page, so it is tested with plain objects.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

/** A colour as hue (0 to 360), saturation and brightness (each 0 to 1). */
export interface Hsv {
  readonly h: number;
  readonly s: number;
  readonly v: number;
}

/**
 * A colour typed as `#rgb` or `#rrggbb`, in either case and with or without its `#`,
 * written as `#rrggbb` in small letters. `null` when it is no such colour.
 */
export function parseHex(typed: string): string | null {
  const found = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(typed.trim());
  if (!found?.[1]) return null;
  const digits = found[1].toLowerCase();
  const long = digits.length === 3 ? [...digits].map((digit) => digit + digit).join("") : digits;
  return `#${long}`;
}

/** A colour written as `#rrggbb` as hue, saturation and brightness. */
export function hexToHsv(hex: string): Hsv {
  const value = Number.parseInt(hex.slice(1), 16);
  const red = ((value >> 16) & 255) / 255;
  const green = ((value >> 8) & 255) / 255;
  const blue = (value & 255) / 255;
  const most = Math.max(red, green, blue);
  const least = Math.min(red, green, blue);
  const spread = most - least;
  let hue = 0;
  if (spread !== 0) {
    if (most === red) hue = ((green - blue) / spread) % 6;
    else if (most === green) hue = (blue - red) / spread + 2;
    else hue = (red - green) / spread + 4;
  }
  return { h: (hue * 60 + 360) % 360, s: most === 0 ? 0 : spread / most, v: most };
}

/** Hue, saturation and brightness as a colour written as `#rrggbb`. */
export function hsvToHex({ h, s, v }: Hsv): string {
  const channel = (offset: number) => {
    const at = (offset + h / 60) % 6;
    const value = v - v * s * Math.max(0, Math.min(at, 4 - at, 1));
    return Math.round(value * 255)
      .toString(16)
      .padStart(2, "0");
  };
  return `#${channel(5)}${channel(3)}${channel(1)}`;
}

/** A colour the theme in use gives a design token, as `#rrggbb`, if it is one. */
export function tokenHex(token: string): string | null {
  const held = getComputedStyle(document.documentElement).getPropertyValue(token).trim();
  // A theme may give the colour with how opaque it is after it.
  return parseHex(held.length === 9 ? held.slice(0, 7) : held);
}

/** Where a picker starts when there is no colour to start from: the theme's accent. */
export function startingHex(): string {
  return tokenHex("--lume-color-accent") ?? hsvToHex({ h: 220, s: 0.5, v: 1 });
}

/** Whether what is typed in the field should be said to be wrong already. */
export function looksWrong(typed: string, tried: boolean): boolean {
  if (parseHex(typed) !== null) return false;
  return tried || /[^0-9a-f#]/i.test(typed) || typed.replace(/^#/, "").length > 6;
}

/** What a picker is asked for with. */
export interface ColorAsk {
  /** What the colour is for, said beside the picker's name: "Tab · build". */
  readonly label: string;
  /** The colour to start from, as `#rrggbb`. */
  readonly value: string;
  /** Where the control that asked is on the page, which the picker opens from. */
  readonly anchor: {
    readonly left: number;
    readonly top: number;
    readonly bottom: number;
    readonly width: number;
  };
  /** Whether it was asked for from the keyboard, which opens it at once. */
  readonly keyboard: boolean;
  /** What is done with the colour once it is applied, as `#rrggbb`. */
  apply(hex: string): void;
  /** Gives the keyboard back to where it was when the picker is put away. */
  readonly returnFocus?: () => void;
}

/** The picker a window has open, if it has one. */
export interface ColorPickerState {
  readonly asked: ColorAsk | null;
  open(ask: ColorAsk): void;
  close(): void;
}

/** The picker of this page: a window has one at a time. */
export const colorPicker: StoreApi<ColorPickerState> = createStore<ColorPickerState>((set) => ({
  asked: null,
  open: (ask) => set({ asked: ask }),
  close: () => set({ asked: null }),
}));

/** How wide the picker is, and how tall, in pixels. */
const PICKER_WIDTH_PX = 248;
const PICKER_HEIGHT_PX = 318;

/** How far the picker keeps from the edges of the window and from its control. */
const PICKER_MARGIN_PX = 8;

/** How far from its left edge the picker's point of origin is under its control. */
const PICKER_ORIGIN_INSET_PX = 24;

/**
 * Where the picker goes for a control at `anchor` in a window of `width` by `height`:
 * below the control if it fits there, otherwise above, and never past an edge. The
 * origin is the point it grows from, under the middle of the control.
 */
export function pickerPlace(
  anchor: ColorAsk["anchor"],
  width: number,
  height: number,
): { left: number; top: number; originX: number; below: boolean } {
  const middle = anchor.left + anchor.width / 2;
  const left = Math.max(
    PICKER_MARGIN_PX,
    Math.min(width - PICKER_WIDTH_PX - PICKER_MARGIN_PX, middle - PICKER_ORIGIN_INSET_PX),
  );
  const below = anchor.bottom + PICKER_MARGIN_PX + PICKER_HEIGHT_PX < height;
  const top = below
    ? anchor.bottom + PICKER_MARGIN_PX
    : Math.max(PICKER_MARGIN_PX, anchor.top - PICKER_MARGIN_PX - PICKER_HEIGHT_PX);
  return { left, top, originX: Math.round(middle - left), below };
}

/**
 * Asks the page's picker for a colour, from the control `from` that was pressed.
 * `event` says whether it was pressed from the keyboard.
 */
export function askColor(
  from: HTMLElement,
  event: { readonly detail: number },
  ask: Pick<ColorAsk, "label" | "value" | "apply" | "returnFocus">,
): void {
  const rect = from.getBoundingClientRect();
  colorPicker.getState().open({
    returnFocus: () => {
      // A control in a menu is gone once the menu is put away.
      if (from.isConnected) from.focus();
    },
    ...ask,
    anchor: { left: rect.left, top: rect.top, bottom: rect.bottom, width: rect.width },
    keyboard: event.detail === 0,
  });
}
