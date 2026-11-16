/**
 * One setting as a row: what it is called and what is said of it at the left, and one
 * control at the right.
 *
 * A control shows what the configuration in use says and nothing of its own. Using it
 * asks for the user's file to be changed; the row changes when the configuration
 * comes back changed, so a setting that could not be written is never shown as if it
 * had been. Each control is the element a browser already has for the job, named
 * after its row.
 */

import { useEffect, useId, useRef, type ReactNode } from "react";

import { ChevronDownIcon } from "../shell/icons.tsx";
import {
  chosenIndex,
  writeChoice,
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
  onPress,
}: {
  children: ReactNode;
  quiet?: boolean;
  primary?: boolean;
  onPress: () => void;
}) {
  return (
    <button
      type="button"
      className="settings-button"
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

function Select({ row, labelled, write }: ControlProps) {
  const chosen = chosenIndex(row);
  return (
    <span className="settings-select">
      <select
        aria-labelledby={labelled}
        // The choices are told apart by their places: their values are of any kind.
        value={chosen < 0 ? "" : String(chosen)}
        onClick={(event) => event.stopPropagation()}
        onChange={(event) => {
          const choice = row.choices[Number(event.target.value)];
          if (choice) write(writeChoice(row, choice));
        }}
      >
        {chosen < 0 && <option value="" />}
        {row.choices.map((choice, index) => (
          <option key={index} value={String(index)}>
            {choice.label}
          </option>
        ))}
      </select>
      <ChevronDownIcon />
    </span>
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
        <div id={label}>{row.label}</div>
        {row.help !== undefined && <div>{row.help}</div>}
        {row.overridden && <div className="settings-note">{overriddenNote(row)}</div>}
        {wrong !== undefined && (
          <div className="settings-problem" role="alert">
            {wrong}
          </div>
        )}
      </div>
      {row.control === "switch" && <Switch {...control} />}
      {row.control === "select" && <Select {...control} />}
      {row.control === "segmented" && <Segmented {...control} />}
      {row.control === "text" && <TextField {...control} />}
    </div>
  );
}
