/**
 * One setting as a row: what it is called and what is said of it at the left, and one
 * control at the right.
 *
 * A control shows what the configuration in use says and nothing of its own. Using it
 * asks for the user's file to be changed; the row changes when the configuration
 * comes back changed, so a setting that could not be written is never shown as if it
 * had been. Each control is the element a browser already has for the job, named
 * after its row; a choice among many is the window's own, as the list a browser drops
 * for one is the system's and not the theme's.
 */

import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from "react";

import { lookup } from "../lookup.ts";
import { askColor, startingHex } from "../shell/color-picker.ts";
import { AgainIcon } from "../shell/icons.tsx";
import { OwnColorSwatch } from "../shell/OwnColorSwatch.tsx";
import { roveWithin } from "../shell/roving.ts";
import { Select } from "./Select.tsx";
import {
  chosenIndex,
  shownNumber,
  stepNumber,
  typedColor,
  writeChoice,
  writeNumber,
  writeSwatch,
  writeSwitch,
  writeText,
  type SettingRow,
  type SettingWrite,
} from "./model.ts";

/** A button of the settings window. `quiet` is one that is only words until pointed at. */
export function Button({
  children,
  quiet = false,
  primary = false,
  label,
  onPress,
}: {
  children: ReactNode;
  quiet?: boolean;
  primary?: boolean;
  /** What it does, where its words alone do not tell it from another. */
  label?: string;
  onPress: () => void;
}) {
  return (
    <button
      type="button"
      className="settings-button"
      aria-label={label}
      data-quiet={quiet}
      data-primary={primary}
      onClick={(event) => {
        // A row that is itself pressed, as a search result is, is not pressed with it.
        event.stopPropagation();
        onPress();
      }}
    >
      {children}
    </button>
  );
}

interface ControlProps {
  row: SettingRow;
  /** The id of the element that names the row. */
  labelled: string;
  write: (change: SettingWrite) => void;
}

function Switch({ row, labelled, write }: ControlProps) {
  const on = row.effective === true;
  return (
    <button
      type="button"
      role="switch"
      className="settings-switch"
      aria-checked={on}
      aria-labelledby={labelled}
      onClick={(event) => {
        event.stopPropagation();
        write(writeSwitch(row, !on));
      }}
    />
  );
}

function Choice({ row, labelled, write }: ControlProps) {
  return (
    <Select
      // The choices are told apart by their places: their values are of any kind.
      choices={row.choices.map((choice) => choice.label)}
      chosen={chosenIndex(row)}
      labelled={labelled}
      onChoose={(index) => {
        const choice = row.choices[index];
        if (choice) write(writeChoice(row, choice));
      }}
    />
  );
}

function Segmented({ row, labelled, write }: ControlProps) {
  const chosen = chosenIndex(row);
  return (
    <div className="settings-segmented" role="group" aria-labelledby={labelled}>
      {row.choices.map((choice, index) => (
        <button
          key={index}
          type="button"
          aria-pressed={index === chosen}
          onClick={(event) => {
            event.stopPropagation();
            if (index !== chosen) write(writeChoice(row, choice));
          }}
        >
          {choice.label}
        </button>
      ))}
    </div>
  );
}

function TextField({ row, labelled, write }: ControlProps) {
  const field = useRef<HTMLInputElement>(null);
  const held = typeof row.value === "string" ? row.value : "";
  // The field shows what is in use whenever that changes, unless it is being typed in.
  useEffect(() => {
    const element = field.current;
    if (element && document.activeElement !== element) element.value = held;
  }, [held]);
  const commit = () => {
    const typed = field.current?.value ?? "";
    if (typed.trim() !== held) write(writeText(row, typed));
  };
  return (
    <input
      ref={field}
      type="text"
      className="settings-text"
      aria-labelledby={labelled}
      defaultValue={held}
      placeholder={row.unsetLabel}
      spellCheck={false}
      autoComplete="off"
      onClick={(event) => event.stopPropagation()}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          commit();
        } else if (event.key === "Escape") {
          // What was typed is given up, and the key goes no further: it does not
          // also close the window.
          event.currentTarget.value = held;
          event.currentTarget.blur();
          event.stopPropagation();
        }
      }}
    />
  );
}

/** How a unit is written after a number in its field: short, as a field has no room. */
const UNIT_SHORT: Readonly<Record<string, string>> = { seconds: "s" };

/**
 * A number that is typed. What is typed is held to the setting's bounds when it is
 * written, so the field shows the nearest value the setting takes once the
 * configuration comes back. The arrow keys move it by its step, ten steps with Shift.
 * While a setting that can be left out is left out, the field is empty and shows what
 * stands in for it; while it is set, a button beside it leaves it out again.
 */
function NumberField({ row, labelled, write }: ControlProps) {
  const field = useRef<HTMLInputElement>(null);
  const shape = row.number;
  const held = shownNumber(row, row.value);
  // The field shows what is in use whenever that changes, unless it is being typed in.
  useEffect(() => {
    const element = field.current;
    if (element && document.activeElement !== element) element.value = held;
  }, [held]);
  if (!shape) return null;

  /** Asks for a change and shows at once the value it asks for. */
  const ask = (change: SettingWrite | null) => {
    const element = field.current;
    if (change === null || (change.op === "set" && change.value === row.value)) {
      if (element) element.value = held;
      return;
    }
    if (change.op === "unset" && row.value === null) return;
    if (element) element.value = change.op === "set" ? shownNumber(row, change.value) : "";
    write(change);
  };
  const commit = () => {
    const typed = field.current?.value ?? "";
    if (typed.trim() !== held) ask(writeNumber(row, typed));
  };
  const unit =
    shape.unit === undefined ? undefined : (lookup(UNIT_SHORT, shape.unit) ?? shape.unit);
  const now = typeof row.effective === "number" ? row.effective * shape.scale : undefined;
  return (
    <span className="settings-number-group">
      {row.unsetLabel !== undefined && row.value !== null && (
        <button
          type="button"
          className="settings-reset"
          aria-label={`Use the ${row.unsetLabel.toLowerCase()}`}
          title={`Use the ${row.unsetLabel.toLowerCase()}`}
          onClick={(event) => {
            event.stopPropagation();
            ask({ op: "unset", path: row.path });
          }}
        >
          <AgainIcon />
        </button>
      )}
      <span
        className="settings-number"
        onClick={(event) => {
          event.stopPropagation();
          field.current?.focus();
        }}
      >
        <input
          ref={field}
          type="text"
          inputMode="decimal"
          role="spinbutton"
          aria-labelledby={labelled}
          aria-valuemin={Number.isFinite(shape.min) ? shape.min : undefined}
          aria-valuemax={Number.isFinite(shape.max) ? shape.max : undefined}
          aria-valuenow={now}
          defaultValue={held}
          placeholder={shownNumber(row, row.value === null ? row.effective : null)}
          spellCheck={false}
          autoComplete="off"
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              commit();
            } else if (event.key === "Escape") {
              // What was typed is given up, and the key goes no further: it does not
              // also close the window.
              event.currentTarget.value = held;
              event.currentTarget.blur();
              event.stopPropagation();
            } else if (event.key === "ArrowUp" || event.key === "ArrowDown") {
              event.preventDefault();
              const typed = event.currentTarget.value.trim().replace(",", ".");
              const from = typed === "" || Number.isNaN(Number(typed)) ? null : Number(typed);
              const by = (event.key === "ArrowUp" ? 1 : -1) * (event.shiftKey ? 10 : 1);
              ask(stepNumber(row, from, by));
            }
          }}
        />
        {unit !== undefined && (
          <span className="settings-number-unit" aria-hidden="true">
            {unit}
          </span>
        )}
      </span>
    </span>
  );
}

/**
 * Gives a swatch its colour through its style object, as the page's policy allows,
 * rather than through an attribute of the markup.
 */
function paint(color: string | null): (element: HTMLElement | null) => void {
  return (element) => {
    if (!element) return;
    if (color === null) element.style.removeProperty("--swatch");
    else element.style.setProperty("--swatch", color);
  };
}

/**
 * A colour: the theme's own, one of the row's swatches, or one of the user's own. The
 * swatches are one stop for the Tab key and the arrows choose among them, as radio
 * buttons do. The last swatch opens the colour picker, which takes a colour mixed on
 * it or typed as `#rgb` or `#rrggbb`, and writes it only when it is applied.
 */
function Swatches({ row, labelled, write }: ControlProps) {
  const swatches = row.swatches ?? [];
  const current = typeof row.value === "string" ? row.value : null;
  const preset = swatches.find((swatch) => swatch.value === current);
  const own = current !== null && preset === undefined ? typedColor(current) : null;
  const shown = own ?? preset?.color ?? (typeof row.effective === "string" ? row.effective : "");

  const choose = (value: string | null) => {
    if (value !== current) write(writeSwatch(row, value));
  };
  const rove = (event: KeyboardEvent<HTMLElement>) => {
    const group = event.currentTarget.parentElement;
    if (!group || !roveWithin(event, group, "[role=radio]", "horizontal")) return;
    event.preventDefault();
    const reached = document.activeElement;
    // The swatch of the user's own takes the keyboard only: pressed, it opens the picker.
    if (reached instanceof HTMLElement && reached.dataset["ownColor"] === undefined) {
      reached.click();
    }
  };
  const pressed = current === null ? "theme" : own !== null ? "own" : current;
  return (
    <div className="settings-swatches">
      <div className="settings-swatch-row" role="radiogroup" aria-labelledby={labelled}>
        <button
          type="button"
          role="radio"
          className="settings-swatch-theme"
          aria-checked={pressed === "theme"}
          tabIndex={pressed === "theme" ? 0 : -1}
          title={row.unsetLabel}
          onClick={(event) => {
            event.stopPropagation();
            choose(null);
          }}
          onKeyDown={rove}
        >
          {current === null && shown !== "" && (
            <span className="settings-swatch-dot" ref={paint(shown)} aria-hidden="true" />
          )}
          Theme
        </button>
        {swatches.map((swatch) => (
          <button
            key={swatch.value}
            type="button"
            role="radio"
            className="settings-swatch"
            aria-checked={pressed === swatch.value}
            aria-label={swatch.label}
            title={swatch.label}
            tabIndex={pressed === swatch.value ? 0 : -1}
            ref={paint(swatch.color)}
            onClick={(event) => {
              event.stopPropagation();
              choose(swatch.value);
            }}
            onKeyDown={rove}
          />
        ))}
        <OwnColorSwatch
          className="settings-swatch-own"
          size={18}
          color={own}
          onKeyDown={rove}
          onChoose={(element, event) =>
            askColor(element, event, {
              label: row.label,
              value: typedColor(shown) ?? swatches[0]?.color ?? startingHex(),
              apply: (color) => choose(color),
            })
          }
        />
      </div>
    </div>
  );
}

/** Where a setting that the user's file does not decide gets its value. */
function overriddenNote(row: SettingRow): string {
  return row.source === "workspace"
    ? "Set by this workspace’s file, which comes before yours."
    : "Set by the profile in use, which comes before your file.";
}

/**
 * One row. `failure` is why the last change asked for here was not made, `flash` marks
 * a row a search led to, and `open` makes the whole row something to press, as a
 * search result is.
 */
export function Row({
  row,
  write,
  failure,
  flash = false,
  flashed,
  open,
}: {
  row: SettingRow;
  write: (change: SettingWrite) => void;
  failure?: string | undefined;
  flash?: boolean;
  /** The row has shown that it is the one a search led to. */
  flashed?: () => void;
  open?: (() => void) | undefined;
}) {
  const label = useId();
  const element = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (flash) element.current?.scrollIntoView({ block: "nearest" });
  }, [flash]);
  const control = { row, labelled: label, write };
  const wrong = failure ?? row.problem;
  return (
    <div
      ref={element}
      className="settings-row"
      data-path={row.path}
      data-flash={flash}
      data-hit={open !== undefined}
      onClick={open}
      onAnimationEnd={flashed}
    >
      <div className="settings-label">
        <div id={label}>
          {row.label}
          {row.plugin !== undefined && <span className="settings-from">from {row.plugin}</span>}
        </div>
        {row.help !== undefined && <div>{row.help}</div>}
        {row.overridden && <div className="settings-note">{overriddenNote(row)}</div>}
        {wrong !== undefined && (
          <div className="settings-problem" role="alert">
            {wrong}
          </div>
        )}
      </div>
      {row.control === "switch" && <Switch {...control} />}
      {row.control === "select" && <Choice {...control} />}
      {row.control === "segmented" && <Segmented {...control} />}
      {row.control === "text" && <TextField {...control} />}
      {row.control === "number" && <NumberField {...control} />}
      {row.control === "swatch" && <Swatches {...control} />}
    </div>
  );
}
