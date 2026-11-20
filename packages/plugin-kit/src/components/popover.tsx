/**
 * Things that open over the view: a popover beside what opened it, a menu in one, and a
 * button with a menu of its other ways attached.
 *
 * A view cannot draw outside its own frame, so a popover stays inside it: it opens
 * below or above its anchor, from its left or right edge, where the view has room.
 */

import {
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import { Button, type ButtonProps } from "./controls";
import { ChevronDown, cx } from "./icons";

export interface PopoverProps {
  readonly open: boolean;
  /** Called when it should close: a press outside it, or Escape. */
  readonly onClose: () => void;
  /** What it opens beside; it is drawn in the same box. */
  readonly anchor: ReactNode;
  /** Which edge of the anchor it lines up with. */
  readonly align?: "start" | "end" | undefined;
  /** Whether it opens below the anchor or above it. */
  readonly side?: "bottom" | "top" | undefined;
  readonly label?: string | undefined;
  readonly className?: string | undefined;
  readonly children?: ReactNode;
}

/** A small box of controls that opens beside what opened it. */
export function Popover({
  open,
  onClose,
  anchor,
  align = "start",
  side = "bottom",
  label,
  className,
  children,
}: PopoverProps) {
  const box = useRef<HTMLDivElement>(null);
  const wrapper = useRef<HTMLSpanElement>(null);
  const closing = useRef(onClose);
  closing.current = onClose;

  useEffect(() => {
    if (!open) return undefined;
    const pressed = (event: PointerEvent) => {
      if (event.target instanceof Node && !wrapper.current?.contains(event.target)) {
        closing.current();
      }
    };
    // A press anywhere else in the window never reaches the view; the keyboard
    // leaving it is all it hears of one.
    const left = () => closing.current();
    document.addEventListener("pointerdown", pressed, true);
    window.addEventListener("blur", left);
    return () => {
      document.removeEventListener("pointerdown", pressed, true);
      window.removeEventListener("blur", left);
    };
  }, [open]);

  useLayoutEffect(() => {
    if (!open) return;
    const first = box.current?.querySelector<HTMLElement>(
      "[autofocus], button, [href], input, select, textarea, [tabindex]:not([tabindex='-1'])",
    );
    (first ?? box.current)?.focus();
  }, [open]);

  return (
    <span className="lk-anchor" ref={wrapper}>
      {anchor}
      {open && (
        <div
          ref={box}
          role="dialog"
          aria-label={label}
          tabIndex={-1}
          className={cx("lk-popover", `lk-align-${align}`, `lk-side-${side}`, className)}
          onKeyDown={(event) => {
            if (event.key !== "Escape") return;
            event.preventDefault();
            event.stopPropagation();
            onClose();
            wrapper.current?.querySelector<HTMLElement>("button")?.focus();
          }}
        >
          {children}
        </div>
      )}
    </span>
  );
}

/** One row of a menu, or a line between rows. */
export type MenuItem =
  | {
      readonly id: string;
      readonly label: ReactNode;
      /** Words at the row's end: a key, a count. */
      readonly hint?: ReactNode | undefined;
      readonly disabled?: boolean | undefined;
      readonly danger?: boolean | undefined;
      /** A tick before it, for a row that is one of several choices. */
      readonly checked?: boolean | undefined;
    }
  | { readonly separator: true };

export interface MenuProps {
  readonly items: readonly MenuItem[];
  readonly onSelect: (id: string) => void;
  readonly label: string;
  readonly className?: string | undefined;
}

/** The rows of a menu. Put it in a `Popover`, or use `MenuButton` for both. */
export function Menu({ items, onSelect, label, className }: MenuProps) {
  const list = useRef<HTMLDivElement>(null);
  const move = (event: KeyboardEvent, by: number | "first" | "last") => {
    const rows = [
      ...(list.current?.querySelectorAll<HTMLElement>("[role^=menuitem]:not(:disabled)") ?? []),
    ];
    if (rows.length === 0) return;
    const at = rows.indexOf(document.activeElement as HTMLElement);
    const next =
      by === "first" ? 0 : by === "last" ? rows.length - 1 : (at + by + rows.length) % rows.length;
    rows[next]?.focus();
    event.preventDefault();
  };
  return (
    <div
      ref={list}
      role="menu"
      aria-label={label}
      className={cx("lk-menu", className)}
      onKeyDown={(event) => {
        if (event.key === "ArrowDown") move(event, 1);
        else if (event.key === "ArrowUp") move(event, -1);
        else if (event.key === "Home") move(event, "first");
        else if (event.key === "End") move(event, "last");
      }}
    >
      {items.map((item, index) =>
        "separator" in item ? (
          <div key={`separator-${index}`} role="separator" className="lk-menu-separator" />
        ) : (
          <button
            key={item.id}
            type="button"
            role={item.checked === undefined ? "menuitem" : "menuitemradio"}
            aria-checked={item.checked}
            disabled={item.disabled}
            className={cx("lk-menu-item", item.danger && "lk-danger")}
            onClick={() => onSelect(item.id)}
          >
            <span className="lk-menu-label">{item.label}</span>
            {item.hint !== undefined && <span className="lk-menu-hint">{item.hint}</span>}
          </button>
        ),
      )}
    </div>
  );
}

export interface MenuButtonProps extends Omit<ButtonProps, "onSelect"> {
  readonly items: readonly MenuItem[];
  readonly onSelect: (id: string) => void;
  /** What the menu is, for assistive technology. */
  readonly menuLabel: string;
  readonly align?: "start" | "end" | undefined;
  readonly side?: "bottom" | "top" | undefined;
}

/** A button that opens a menu. */
export function MenuButton({
  items,
  onSelect,
  menuLabel,
  align,
  side,
  children,
  ...button
}: MenuButtonProps) {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <Popover
      open={open}
      onClose={() => setOpen(false)}
      align={align}
      side={side}
      label={menuLabel}
      className="lk-popover-menu"
      anchor={
        <Button
          {...button}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-controls={open ? id : undefined}
          onClick={() => setOpen((now) => !now)}
        >
          {children}
        </Button>
      }
    >
      <div id={id}>
        <Menu
          items={items}
          label={menuLabel}
          onSelect={(chosen) => {
            setOpen(false);
            onSelect(chosen);
          }}
        />
      </div>
    </Popover>
  );
}

export interface SplitButtonProps extends Omit<ButtonProps, "onSelect"> {
  /** The other ways of doing what the button does. */
  readonly items: readonly MenuItem[];
  readonly onSelect: (id: string) => void;
  readonly menuLabel: string;
  readonly align?: "start" | "end" | undefined;
  readonly side?: "bottom" | "top" | undefined;
}

/**
 * A button with its other ways in a menu beside it: "Commit", and under the arrow
 * "Commit and push", "Amend".
 */
export function SplitButton({
  items,
  onSelect,
  menuLabel,
  align = "end",
  side,
  className,
  ...button
}: SplitButtonProps) {
  const [open, setOpen] = useState(false);
  return (
    <span className={cx("lk-split-button", className)}>
      <Button {...button} />
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        align={align}
        side={side}
        label={menuLabel}
        className="lk-popover-menu"
        anchor={
          <Button
            variant={button.variant}
            size={button.size}
            disabled={button.disabled}
            aria-label={menuLabel}
            aria-haspopup="menu"
            aria-expanded={open}
            className="lk-split-arrow"
            onClick={() => setOpen((now) => !now)}
          >
            <ChevronDown />
          </Button>
        }
      >
        <Menu
          items={items}
          label={menuLabel}
          onSelect={(chosen) => {
            setOpen(false);
            onSelect(chosen);
          }}
        />
      </Popover>
    </span>
  );
}
