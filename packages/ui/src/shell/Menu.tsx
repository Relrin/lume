/**
 * A small menu that drops from the control that opened it.
 *
 * The menu takes the keyboard while it is open: the arrow keys move through its items,
 * Enter and Space choose the one that has the keyboard, and Escape, Tab or a press
 * anywhere else puts the menu away. It is rendered beside its control, inside an
 * element the control's own styles position it against.
 */

import { Fragment, useEffect, useRef } from "react";

import { CheckIcon } from "./icons.tsx";

/** One thing a menu offers. */
export interface MenuItem {
  readonly key: string;
  readonly label: string;
  /** Shown at the item's far end, in a quieter colour: a key, or a few words. */
  readonly hint?: string;
  /** Whether the item is the one of its kind in effect. Given at all, the item is one of a choice. */
  readonly checked?: boolean;
  readonly disabled?: boolean;
  /** Whether a line separates the item from the ones before it. */
  readonly separated?: boolean;
  /** What choosing the item does. The menu is put away first. */
  choose(): void;
}

/**
 * A menu on show. `close` puts it away and says where the keyboard goes then. A menu
 * that is one of several in a row is given `side`, which shows the menu before it
 * (-1) or after it (1) when the left or the right arrow is pressed.
 */
export function Menu({
  label,
  items,
  close,
  side,
}: {
  label: string;
  items: readonly MenuItem[];
  close: () => void;
  side?: (by: -1 | 1) => void;
}) {
  const menu = useRef<HTMLDivElement>(null);
  // Room for a tick is kept only in a menu that has something to tick.
  const choice = items.some((item) => item.checked !== undefined);

  const buttons = (): HTMLButtonElement[] => [
    ...(menu.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []),
  ];

  // The keyboard goes to the item in effect, or the first, when the items arrive.
  const count = items.length;
  useEffect(() => {
    const all = buttons();
    const inEffect = all.find((button) => button.getAttribute("aria-checked") === "true");
    if (inEffect) inEffect.focus();
    else if (!menu.current?.contains(document.activeElement)) all[0]?.focus();
  }, [count]);

  const step = (by: number) => {
    const all = buttons();
    if (all.length === 0) return;
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const next = at < 0 ? (by > 0 ? 0 : all.length - 1) : (at + by + all.length) % all.length;
    all[next]?.focus();
  };

  return (
    <>
      <div className="menu-backdrop" onPointerDown={close} />
      <div
        ref={menu}
        className="menu"
        role="menu"
        aria-label={label}
        tabIndex={-1}
        onKeyDown={(event) => {
          switch (event.key) {
            case "ArrowDown":
              step(1);
              break;
            case "ArrowUp":
              step(-1);
              break;
            case "Home":
              buttons()[0]?.focus();
              break;
            case "End":
              buttons().at(-1)?.focus();
              break;
            case "ArrowLeft":
            case "ArrowRight":
              side?.(event.key === "ArrowLeft" ? -1 : 1);
              break;
            case "Escape":
            case "Tab":
              close();
              break;
            default:
              // Keys pressed here are for the menu, not for the terminal behind it.
              event.stopPropagation();
              return;
          }
          event.preventDefault();
          event.stopPropagation();
        }}
      >
        {items.map((item) => (
          <Fragment key={item.key}>
            {item.separated && <div className="menu-separator" role="separator" />}
            <button
              type="button"
              className="menu-item"
              role={item.checked === undefined ? "menuitem" : "menuitemradio"}
              aria-checked={item.checked}
              disabled={item.disabled}
              tabIndex={-1}
              onClick={() => {
                close();
                item.choose();
              }}
            >
              {choice && <span className="menu-check">{item.checked && <CheckIcon />}</span>}
              <span className="menu-label">{item.label}</span>
              {item.hint !== undefined && <span className="menu-hint">{item.hint}</span>}
            </button>
          </Fragment>
        ))}
      </div>
    </>
  );
}
