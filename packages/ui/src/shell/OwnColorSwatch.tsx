/**
 * The swatch that opens the colour picker: a wheel of hues with a plus while the user
 * has no colour of their own, their colour once they have one. It sits last in a row
 * of swatches and is a radio of the row, checked while their colour is the one in use.
 */

import type { KeyboardEvent, MouseEvent } from "react";

export function OwnColorSwatch({
  color,
  size,
  className,
  role = "radio",
  onChoose,
  onKeyDown,
}: {
  /** The user's own colour in use, as `#rrggbb`, or nothing. */
  color: string | null;
  /** How wide the swatch is, in pixels. */
  size: number;
  /** The class the row's other swatches have, which the row's arrow keys look for. */
  className?: string;
  role?: "radio" | "menuitemradio";
  /** Opens the picker: the swatch, and the press, which says whether it was a key's. */
  onChoose: (element: HTMLElement, event: MouseEvent<HTMLElement>) => void;
  onKeyDown?: (event: KeyboardEvent<HTMLElement>) => void;
}) {
  const label = color === null ? "Custom colour…" : `Custom colour ${color} · change…`;
  return (
    <button
      type="button"
      role={role}
      className={className === undefined ? "color-wheel" : `${className} color-wheel`}
      data-own-color=""
      data-own={color === null ? undefined : true}
      aria-checked={color !== null}
      aria-label={label}
      title={label}
      tabIndex={role === "radio" ? (color !== null ? 0 : -1) : undefined}
      // Through the element's style object, not an attribute, as the page's policy asks.
      ref={(element) => {
        if (!element) return;
        element.style.width = `${size}px`;
        element.style.height = `${size}px`;
        if (color === null) element.style.removeProperty("--own");
        else element.style.setProperty("--own", color);
      }}
      onClick={(event) => {
        event.stopPropagation();
        onChoose(event.currentTarget, event);
      }}
      onKeyDown={onKeyDown}
    >
      <svg
        width={Math.round(size * 0.55)}
        height={Math.round(size * 0.55)}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
        aria-hidden="true"
      >
        <path d="M12 6v12M6 12h12" />
      </svg>
    </button>
  );
}
