/**
 * A small menu that drops from the control that opened it.
 *
 * The menu takes the keyboard while it is open: the arrow keys move through its items,
 * Enter and Space choose the one that has the keyboard, and Escape, Tab or a press
 * anywhere else puts the menu away. It is rendered beside its control, inside an
 * element the control's own styles position it against.
 *
 * An item may hold a menu of its own, which opens beside the item when the pointer
 * rests on it, when it is chosen, or when the right arrow is pressed on it. The left
 * arrow or Escape there goes back to the item.
 */

import { Fragment, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";

import { CheckIcon, ChevronRightIcon } from "./icons.tsx";

/** One thing a menu offers. */
export interface MenuItem {
  readonly key: string;
  readonly label: string;
  /** Shown at the item's far end, in a quieter colour: a key, or a few words. */
  readonly hint?: string;
  /** A small picture before the label. */
  readonly icon?: ReactNode;
  /** Whether the item is the one of its kind in effect. Given at all, the item is one of a choice. */
  readonly checked?: boolean;
  readonly disabled?: boolean;
  /** Whether a line separates the item from the ones before it. */
  readonly separated?: boolean;
  /** The menu the item opens beside it. An item with one is not chosen itself. */
  readonly items?: readonly MenuItem[];
  /** What choosing the item does. The menu is put away first. */
  choose(): void;
}

/**
 * A menu on show. `close` puts it away and says where the keyboard goes then. A menu
 * that is one of several in a row is given `side`, which shows the menu before it
 * (-1) or after it (1) when the left or the right arrow is pressed. A menu that opened
 * from an item of another is given `back`, which goes back to that item.
 */
export function Menu({
  label,
  items,
  close,
  side,
  back,
  keyboard = true,
}: {
  label: string;
  items: readonly MenuItem[];
  close: () => void;
  side?: (by: -1 | 1) => void;
  back?: () => void;
  /** Whether the menu takes the keyboard as it opens: not when the pointer opened it. */
  keyboard?: boolean;
}) {
  const menu = useRef<HTMLDivElement>(null);
  const sub = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState<string | null>(null);
  // Whether the item's menu was opened from the keyboard, or by a press, and so takes it.
  const [keys, setKeys] = useState(false);
  // Room for a tick is kept only in a menu that has something to tick.
  const choice = items.some((item) => item.checked !== undefined);
  const opened = items.find((item) => item.key === open && item.items !== undefined);

  const buttons = (): HTMLButtonElement[] => [
    ...(menu.current?.querySelectorAll<HTMLButtonElement>(":scope > button:not(:disabled)") ?? []),
  ];
  const buttonOf = (key: string) =>
    buttons().find((button) => button.dataset["key"] === key) ?? null;

  // The keyboard goes to the item in effect, or the first, when the items arrive.
  const count = items.length;
  useEffect(() => {
    if (!keyboard) return;
    const all = buttons();
    const inEffect = all.find((button) => button.getAttribute("aria-checked") === "true");
    if (inEffect) inEffect.focus();
    else if (!menu.current?.contains(document.activeElement)) all[0]?.focus();
  }, [count, keyboard]);

  // A menu of an item stands beside the item, level with it. It is not inside this
  // menu, whose own scrolling would cut it off.
  useLayoutEffect(() => {
    const element = sub.current;
    const owner = open === null ? null : buttonOf(open);
    const here = menu.current;
    if (!element || !owner || !here) return;
    element.style.left = `${here.offsetLeft + here.offsetWidth}px`;
    element.style.top = `${here.offsetTop + owner.offsetTop - here.scrollTop - here.clientTop}px`;
  }, [open]);

  const step = (by: number) => {
    const all = buttons();
    if (all.length === 0) return;
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const next = at < 0 ? (by > 0 ? 0 : all.length - 1) : (at + by + all.length) % all.length;
    all[next]?.focus();
  };

  /** The item that has the keyboard, if it is one of this menu's. */
  const focusedItem = () => {
    const key = (document.activeElement as HTMLElement | null)?.dataset?.["key"];
    return items.find((item) => item.key === key);
  };

  /** Goes back from an item's menu to the item. */
  const leave = () => {
    const key = open;
    setOpen(null);
    if (key !== null) buttonOf(key)?.focus();
  };

  return (
    <>
      {back === undefined && <div className="menu-backdrop" onPointerDown={close} />}
      <div
        ref={menu}
        className={back === undefined ? "menu" : "menu menu-sub"}
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
            case "ArrowRight": {
              const item = focusedItem();
              if (item?.items) {
                setKeys(true);
                setOpen(item.key);
              } else side?.(1);
              break;
            }
            case "ArrowLeft":
              if (back) back();
              else side?.(-1);
              break;
            case "Escape":
              if (back) back();
              else close();
              break;
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
              aria-haspopup={item.items ? "menu" : undefined}
              aria-expanded={item.items ? open === item.key : undefined}
              data-key={item.key}
              disabled={item.disabled}
              tabIndex={-1}
              onPointerEnter={(event) => {
                // The keyboard leaves an item's menu that closes under it for this item.
                if (sub.current?.contains(document.activeElement)) event.currentTarget.focus();
                setKeys(false);
                setOpen(item.items ? item.key : null);
              }}
              onClick={() => {
                if (item.items) {
                  setKeys(true);
                  setOpen(item.key);
                  // Open already, from the pointer resting on it: the press takes it the keyboard.
                  sub.current?.querySelector<HTMLButtonElement>(".menu-item")?.focus();
                  return;
                }
                close();
                item.choose();
              }}
            >
              {choice && <span className="menu-check">{item.checked && <CheckIcon />}</span>}
              {item.icon !== undefined && <span className="menu-icon">{item.icon}</span>}
              <span className="menu-label">{item.label}</span>
              {item.hint !== undefined && <span className="menu-hint">{item.hint}</span>}
              {item.items && (
                <span className="menu-more">
                  <ChevronRightIcon />
                </span>
              )}
            </button>
          </Fragment>
        ))}
      </div>
      {opened?.items && (
        <div ref={sub} className="menu-sub-place">
          <Menu
            label={opened.label}
            items={opened.items}
            close={close}
            back={leave}
            keyboard={keys}
          />
        </div>
      )}
    </>
  );
}
