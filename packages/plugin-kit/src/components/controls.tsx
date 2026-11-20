/**
 * The small controls: buttons, a segmented choice, a switch, a three-state checkbox,
 * tabs, a badge and a text area. Each is the plain HTML control it looks like, styled
 * with the window's tokens by `kit.css`, and takes the attributes that control takes.
 */

import type {
  ButtonHTMLAttributes,
  HTMLAttributes,
  ReactNode,
  Ref,
  TextareaHTMLAttributes,
} from "react";

import { CheckMark, cx, DashMark } from "./icons";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /**
   * `default` is outlined; `primary` is the one thing to do next; `quiet` has no
   * outline until pointed at; `accept` and `danger` are tinted for what they do.
   */
  readonly variant?: "default" | "primary" | "quiet" | "accept" | "danger" | undefined;
  readonly size?: "small" | "medium" | undefined;
  readonly ref?: Ref<HTMLButtonElement> | undefined;
}

/** A button with words on it. */
export function Button({ variant = "default", size = "medium", className, ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      {...rest}
      className={cx("lk-button", `lk-button-${variant}`, size === "small" && "lk-small", className)}
    />
  );
}

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** What the button does, for those who cannot see its picture. */
  readonly label: string;
  /** Whether it is held down, for a button that turns something on and off. */
  readonly pressed?: boolean | undefined;
  readonly ref?: Ref<HTMLButtonElement> | undefined;
}

/** A square button with a picture on it: an inline SVG that draws with `currentColor`. */
export function IconButton({ label, pressed, className, ...rest }: IconButtonProps) {
  return (
    <button
      type="button"
      title={label}
      {...rest}
      aria-label={label}
      aria-pressed={pressed}
      className={cx("lk-icon-button", className)}
    />
  );
}

export interface SegmentedOption<T extends string> {
  readonly value: T;
  readonly label: ReactNode;
  /** What the choice is called where its label is a picture. */
  readonly title?: string | undefined;
}

export interface SegmentedProps<T extends string> {
  readonly options: readonly SegmentedOption<T>[];
  readonly value: T;
  readonly onChange: (value: T) => void;
  /** What is being chosen, for assistive technology. */
  readonly label: string;
  /** Whether the choices share the whole width. */
  readonly full?: boolean | undefined;
  readonly className?: string | undefined;
}

/** A few choices side by side, one of them taken. The arrow keys move between them. */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
  label,
  full,
  className,
}: SegmentedProps<T>) {
  const move = (by: number) => {
    const at = options.findIndex((option) => option.value === value);
    const next = options[(at + by + options.length) % options.length];
    if (next) onChange(next.value);
  };
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={cx("lk-segmented", full && "lk-full", className)}
      onKeyDown={(event) => {
        if (event.key === "ArrowRight" || event.key === "ArrowDown") move(1);
        else if (event.key === "ArrowLeft" || event.key === "ArrowUp") move(-1);
        else return;
        event.preventDefault();
      }}
    >
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          title={option.title}
          aria-checked={option.value === value}
          tabIndex={option.value === value ? 0 : -1}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export interface SwitchProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "onChange"> {
  readonly checked: boolean;
  readonly onChange: (checked: boolean) => void;
  /** What it turns on and off, where no visible label names it. */
  readonly label?: string | undefined;
}

/** A switch: something that is on or off, and changes at once. */
export function Switch({ checked, onChange, label, className, ...rest }: SwitchProps) {
  return (
    <button
      type="button"
      {...rest}
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={cx("lk-switch", className)}
      onClick={() => onChange(!checked)}
    >
      <span className="lk-switch-knob" />
    </button>
  );
}

/** What a checkbox shows: ticked, not, or `mixed` where only some of its things are. */
export type Checked = boolean | "mixed";

export interface CheckboxProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "onChange"> {
  readonly checked: Checked;
  /** Called with what it should become: a mixed box becomes ticked. */
  readonly onChange: (checked: boolean) => void;
  readonly label?: string | undefined;
}

/** A checkbox with three states, as a list's heading has for the rows under it. */
export function Checkbox({ checked, onChange, label, className, ...rest }: CheckboxProps) {
  return (
    <button
      type="button"
      {...rest}
      role="checkbox"
      aria-checked={checked}
      aria-label={label}
      className={cx("lk-checkbox", className)}
      onClick={(event) => {
        event.stopPropagation();
        onChange(checked !== true);
      }}
    >
      <span className="lk-checkbox-box">
        {checked === true && <CheckMark size={10} />}
        {checked === "mixed" && <DashMark size={10} />}
      </span>
    </button>
  );
}

export interface TabItem<T extends string> {
  readonly id: T;
  readonly label: ReactNode;
  /** A number shown after the label, as in "Changes (4)". */
  readonly count?: number | undefined;
}

export interface TabsProps<T extends string> {
  readonly tabs: readonly TabItem<T>[];
  readonly value: T;
  readonly onChange: (id: T) => void;
  readonly label: string;
  readonly className?: string | undefined;
}

/** Tabs across the top of a panel. What each tab shows is the view's to draw. */
export function Tabs<T extends string>({ tabs, value, onChange, label, className }: TabsProps<T>) {
  const move = (by: number) => {
    const at = tabs.findIndex((tab) => tab.id === value);
    const next = tabs[(at + by + tabs.length) % tabs.length];
    if (next) onChange(next.id);
  };
  return (
    <div
      role="tablist"
      aria-label={label}
      className={cx("lk-tabs", className)}
      onKeyDown={(event) => {
        if (event.key === "ArrowRight") move(1);
        else if (event.key === "ArrowLeft") move(-1);
        else return;
        event.preventDefault();
      }}
    >
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={tab.id === value}
          tabIndex={tab.id === value ? 0 : -1}
          onClick={() => onChange(tab.id)}
        >
          {tab.label}
          {tab.count !== undefined && <span className="lk-tab-count"> ({tab.count})</span>}
        </button>
      ))}
    </div>
  );
}

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  /** What the badge is about: its colour. `neutral` unless told otherwise. */
  readonly tone?: "neutral" | "info" | "success" | "warning" | "danger" | "accent" | undefined;
  /** Outlined instead of filled, as a branch's name is beside a commit. */
  readonly outline?: boolean | undefined;
}

/** A word or a number in a small rounded box. */
export function Badge({ tone = "neutral", outline, className, ...rest }: BadgeProps) {
  return (
    <span
      {...rest}
      className={cx("lk-badge", `lk-tone-${tone}`, outline && "lk-outline", className)}
    />
  );
}

export interface TextAreaProps extends TextareaHTMLAttributes<HTMLTextAreaElement> {
  /** Drawn in the terminal's font, as a commit message is. */
  readonly mono?: boolean | undefined;
  readonly ref?: Ref<HTMLTextAreaElement> | undefined;
}

/** A field for several lines of text. */
export function TextArea({ mono, className, ...rest }: TextAreaProps) {
  return <textarea {...rest} className={cx("lk-textarea", mono && "lk-mono", className)} />;
}
