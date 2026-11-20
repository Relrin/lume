/**
 * A choice of one among several: a control that says which is chosen and drops a list
 * of them all when it is pressed.
 *
 * The list is the window's own and not the system's, so it is drawn by the theme like
 * the menus are. It unfolds from the control, downwards, or upwards where there is more
 * room above, and folds away again; it is put in the window itself and not beside the
 * control, so nothing the control is inside of cuts it off.
 *
 * The keyboard stays on the control. The arrow keys open the list and move through it,
 * Home and End go to its ends, Enter and Space choose, Escape and Tab put it away, and
 * a letter goes to the next choice that starts with it.
 */

import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
} from "react";
import { createPortal } from "react-dom";

import { CheckIcon, ChevronDownIcon } from "../shell/icons.tsx";

/** How far from the window's edges the list is kept, and how far from its control. */
const EDGE_PX = 8;
const GAP_PX = 4;

/** How tall the list may be before it scrolls, how tall one choice is, and the room around them. */
const ROOM_PX = 320;
const OPTION_PX = 30;
const PAD_PX = 5;

/** Where the list is put: against the control's right end, under it or over it. */
interface Place {
  readonly host: Element;
  readonly up: boolean;
  readonly room: number;
  readonly style: CSSProperties;
}

/** Whether an element changes at once, with no transition to wait for. */
function instant(element: HTMLElement | null): boolean {
  if (!element) return true;
  const times = getComputedStyle(element).transitionDuration.split(",");
  return !times.some((time) => parseFloat(time) > 0);
}

export function Select({
  choices,
  chosen,
  labelled,
  onChoose,
}: {
  /** What each choice is called. */
  choices: readonly string[];
  /** The place of the one chosen, or -1 while none of them is. */
  chosen: number;
  /** The id of the element that names the control. */
  labelled: string;
  onChoose: (index: number) => void;
}) {
  const id = useId();
  const button = useRef<HTMLButtonElement>(null);
  const drop = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLDivElement>(null);
  // The list is in the page while it has a place, and unfolded while it is open: it
  // comes in folded and unfolds, and stays until it has folded away.
  const [place, setPlace] = useState<Place | null>(null);
  const [open, setOpen] = useState(false);
  // Unfolded all the way, and so scrolled by hand if it is longer than its room.
  const [settled, setSettled] = useState(false);
  const [active, setActive] = useState(-1);

  const show = () => {
    const control = button.current;
    if (!control || open) return;
    const rect = control.getBoundingClientRect();
    const below = window.innerHeight - rect.bottom - GAP_PX - EDGE_PX;
    const above = rect.top - GAP_PX - EDGE_PX;
    const wanted = Math.min(ROOM_PX, choices.length * OPTION_PX + 2 * PAD_PX);
    const up = below < wanted && above > below;
    const room = Math.max(2 * OPTION_PX, Math.min(ROOM_PX, up ? above : below));
    const style: Record<string, string> = {
      right: `${Math.round(window.innerWidth - rect.right)}px`,
      minWidth: `${Math.round(rect.width)}px`,
      "--select-room": `${Math.round(room)}px`,
    };
    if (up) style["bottom"] = `${Math.round(window.innerHeight - rect.top + GAP_PX)}px`;
    else style["top"] = `${Math.round(rect.bottom + GAP_PX)}px`;
    setPlace({
      host: control.closest(".settings-window") ?? document.body,
      up,
      room,
      style: style as CSSProperties,
    });
    setActive(chosen >= 0 ? chosen : 0);
  };

  const hide = () => {
    if (!open) return;
    setOpen(false);
    setSettled(false);
    if (instant(drop.current)) setPlace(null);
  };

  const choose = (index: number) => {
    hide();
    if (index !== chosen && index >= 0 && index < choices.length) onChoose(index);
  };

  // The list comes in folded, with the choice in effect in the middle of it, and
  // unfolds once the page has seen it folded.
  useLayoutEffect(() => {
    if (!place) return;
    const all = list.current;
    const option = all?.children[active];
    if (all && option instanceof HTMLElement) {
      all.scrollTop = option.offsetTop - (place.room - option.offsetHeight) / 2;
    }
    void drop.current?.offsetHeight;
    setOpen(true);
    setSettled(instant(drop.current));
  }, [place]);

  // The choice the keyboard is at stays in sight.
  useEffect(() => {
    const all = list.current;
    const option = all?.children[active];
    if (!open || !all || !(option instanceof HTMLElement)) return;
    const top = option.offsetTop - PAD_PX;
    const bottom = option.offsetTop + option.offsetHeight + PAD_PX;
    if (top < all.scrollTop) all.scrollTop = top;
    else if (bottom > all.scrollTop + all.clientHeight) all.scrollTop = bottom - all.clientHeight;
  }, [active, open]);

  // A window that changes size leaves the list where its control no longer is.
  useEffect(() => {
    if (!open) return;
    window.addEventListener("resize", hide);
    return () => window.removeEventListener("resize", hide);
  }, [open]);

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    const last = choices.length - 1;
    switch (event.key) {
      case "ArrowDown":
      case "ArrowUp": {
        if (!open) show();
        else {
          const next = active + (event.key === "ArrowDown" ? 1 : -1);
          setActive(Math.max(0, Math.min(last, next)));
        }
        break;
      }
      case "Home":
      case "End":
        if (!open) return;
        setActive(event.key === "Home" ? 0 : last);
        break;
      case "Enter":
      case " ":
        if (open) choose(active);
        else show();
        break;
      case "Escape":
        // Open, the key puts the list away and goes no further: it does not also
        // close the window.
        if (!open) return;
        hide();
        break;
      case "Tab":
        hide();
        return;
      default: {
        if (event.key.length !== 1 || event.ctrlKey || event.metaKey || event.altKey) return;
        const letter = event.key.toLowerCase();
        const from = open ? active : chosen;
        for (let step = 1; step <= choices.length; step++) {
          const at = (from + step + choices.length) % choices.length;
          if (!choices[at]?.toLowerCase().startsWith(letter)) continue;
          if (open) setActive(at);
          else if (at !== chosen) onChoose(at);
          break;
        }
        event.stopPropagation();
        return;
      }
    }
    event.preventDefault();
    event.stopPropagation();
  };

  return (
    <span className="settings-select">
      <button
        ref={button}
        type="button"
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={place ? `${id}-list` : undefined}
        aria-activedescendant={place && active >= 0 ? `${id}-${active}` : undefined}
        aria-labelledby={`${labelled} ${id}-value`}
        onClick={(event) => {
          // A row that is itself pressed, as a search result is, is not pressed with it.
          event.stopPropagation();
          if (open) hide();
          else show();
        }}
        onKeyDown={onKeyDown}
        // Space presses a button as it comes up: here it has already done its work.
        onKeyUp={(event) => {
          if (event.key === " ") event.preventDefault();
        }}
        onBlur={hide}
      >
        <span id={`${id}-value`} className="settings-select-value">
          {choices[chosen] ?? ""}
        </span>
        <ChevronDownIcon />
      </button>
      {place &&
        createPortal(
          <>
            {open && (
              <div
                className="menu-backdrop"
                onPointerDown={hide}
                onClick={(event) => event.stopPropagation()}
              />
            )}
            <div
              ref={drop}
              className="settings-select-drop"
              data-open={open}
              data-up={place.up || undefined}
              data-settled={settled || undefined}
              style={place.style}
              // A press in the list leaves the keyboard on the control.
              onMouseDown={(event) => event.preventDefault()}
              onClick={(event) => event.stopPropagation()}
              onTransitionEnd={(event) => {
                if (event.target !== drop.current || event.propertyName !== "grid-template-rows")
                  return;
                if (open) setSettled(true);
                else setPlace(null);
              }}
            >
              <div
                ref={list}
                id={`${id}-list`}
                className="settings-select-list"
                role="listbox"
                aria-labelledby={labelled}
              >
                {choices.map((choice, index) => (
                  <div
                    key={index}
                    id={`${id}-${index}`}
                    className="settings-select-option"
                    role="option"
                    aria-selected={index === chosen}
                    data-active={index === active || undefined}
                    onPointerMove={() => {
                      if (index !== active) setActive(index);
                    }}
                    onClick={() => choose(index)}
                  >
                    <span className="menu-check">{index === chosen && <CheckIcon />}</span>
                    <span className="menu-label">{choice}</span>
                  </div>
                ))}
              </div>
            </div>
          </>,
          place.host,
        )}
    </span>
  );
}
