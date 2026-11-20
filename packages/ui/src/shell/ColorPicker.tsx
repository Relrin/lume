/**
 * The colour picker: a square of saturation and brightness, a bar of hues, the colour
 * before and after, and a field that takes it typed as `#rgb` or `#rrggbb`.
 *
 * It opens from the swatch that asked for it, and does nothing until Apply is pressed,
 * or Enter in the field: Cancel, Escape or a press outside put it away with everything
 * as it was. A window draws one with `ColorPickerHost`, and any control asks for it
 * with `askColor` (see `color-picker.ts`).
 */

import {
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type KeyboardEvent,
  type PointerEvent,
} from "react";
import { createPortal } from "react-dom";

import { Glyph } from "../icons/Glyph.tsx";
import {
  colorPicker,
  hexToHsv,
  hsvToHex,
  looksWrong,
  parseHex,
  pickerPlace,
  startingHex,
  type ColorAsk,
  type Hsv,
} from "./color-picker.ts";

/** How far an arrow key moves the square's knob, and with Shift held. */
const SQUARE_STEP = 0.02;
const SQUARE_STEP_LARGE = 0.1;

/** How far an arrow key moves the hue, in degrees, and with Shift held. */
const HUE_STEP = 5;
const HUE_STEP_LARGE = 30;

/** The highest hue the bar gives: the right end is the left end again. */
const HUE_MAX = 359.9;

/** The colour on the screen under the pointer, where the webview can pick one. */
interface EyeDropperLike {
  open(): Promise<{ sRGBHex: string }>;
}

function eyeDropper(): (new () => EyeDropperLike) | undefined {
  return (window as unknown as { EyeDropper?: new () => EyeDropperLike }).EyeDropper;
}

function clamp(value: number): number {
  return Math.max(0, Math.min(1, value));
}

/** One open picker: what it was asked for with. */
function Picker({ ask, close }: { ask: ColorAsk; close: () => void }) {
  const start = parseHex(ask.value) ?? startingHex();
  const [hsv, setHsv] = useState<Hsv>(() => hexToHsv(start));
  const [typed, setTyped] = useState(start.slice(1));
  const [tried, setTried] = useState(false);
  const field = useRef<HTMLInputElement>(null);
  const [place] = useState(() => pickerPlace(ask.anchor, window.innerWidth, window.innerHeight));
  const Dropper = eyeDropper();

  const parsed = parseHex(typed);
  const mixed = hsvToHex(hsv);
  const wrong = looksWrong(typed, tried);

  useEffect(() => {
    field.current?.focus();
    field.current?.select();
  }, []);

  const fromHsv = (next: Hsv) => {
    setHsv(next);
    setTyped(hsvToHex(next).slice(1));
    setTried(false);
  };
  const fromHex = (hex: string) => {
    const next = hexToHsv(hex);
    // A grey has no hue of its own: the bar stays where it was.
    setHsv((held) => (next.s === 0 ? { ...next, h: held.h } : next));
  };

  const apply = () => {
    const hex = parseHex(typed);
    if (hex === null) {
      setTried(true);
      field.current?.focus();
      return;
    }
    close();
    ask.apply(hex);
  };

  /** Follows the pointer over the square or the bar until it is let go. */
  const drag = (event: PointerEvent<HTMLDivElement>, part: "square" | "hue") => {
    if (event.button > 0) return;
    event.preventDefault();
    const element = event.currentTarget;
    const rect = element.getBoundingClientRect();
    const at = (x: number, y: number) => {
      const across = clamp((x - rect.left) / rect.width);
      const down = clamp((y - rect.top) / rect.height);
      setHsv((held) => {
        const next =
          part === "square"
            ? { ...held, s: across, v: 1 - down }
            : { ...held, h: Math.min(HUE_MAX, across * 360) };
        setTyped(hsvToHex(next).slice(1));
        return next;
      });
      setTried(false);
    };
    at(event.clientX, event.clientY);
    element.focus({ preventScroll: true });
    element.setPointerCapture(event.pointerId);
    const move = (moved: globalThis.PointerEvent) => at(moved.clientX, moved.clientY);
    const up = () => {
      element.removeEventListener("pointermove", move);
      element.removeEventListener("pointerup", up);
      element.removeEventListener("pointercancel", up);
    };
    element.addEventListener("pointermove", move);
    element.addEventListener("pointerup", up);
    element.addEventListener("pointercancel", up);
  };

  const squareKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const moves: Readonly<Record<string, readonly [number, number]>> = {
      ArrowLeft: [-1, 0],
      ArrowRight: [1, 0],
      ArrowUp: [0, 1],
      ArrowDown: [0, -1],
    };
    const move = moves[event.key];
    if (!move) return;
    event.preventDefault();
    const step = event.shiftKey ? SQUARE_STEP_LARGE : SQUARE_STEP;
    fromHsv({ ...hsv, s: clamp(hsv.s + move[0] * step), v: clamp(hsv.v + move[1] * step) });
  };

  const hueKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const moves: Readonly<Record<string, number>> = {
      ArrowLeft: -1,
      ArrowDown: -1,
      ArrowRight: 1,
      ArrowUp: 1,
    };
    const move = moves[event.key];
    if (move === undefined) return;
    event.preventDefault();
    const step = event.shiftKey ? HUE_STEP_LARGE : HUE_STEP;
    fromHsv({ ...hsv, h: (hsv.h + move * step + 360) % 360 });
  };

  const pick = () => {
    if (!Dropper) return;
    new Dropper().open().then(
      (result) => {
        const hex = parseHex(result.sRGBHex);
        if (hex === null) return;
        setTyped(hex.slice(1));
        setTried(false);
        fromHex(hex);
      },
      // Pressing Escape over the screen gives no colour, and that is all.
      () => undefined,
    );
  };

  // The hue at its fullest, which the square shades and the bar's knob shows.
  const hue = hsvToHex({ h: hsv.h, s: 1, v: 1 });
  return (
    <>
      <div className="color-picker-scrim" aria-hidden="true" onPointerDown={close} />
      <div
        className="color-picker"
        role="dialog"
        aria-label={`Custom colour for ${ask.label}`}
        data-animate={!ask.keyboard || undefined}
        data-below={place.below}
        style={{
          left: place.left,
          top: place.top,
          transformOrigin: `${place.originX}px ${place.below ? "0" : "100%"}`,
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            close();
          }
        }}
      >
        <div className="color-picker-head">
          <b>Custom colour</b>
          <small>{ask.label}</small>
        </div>
        <div
          className="color-picker-square"
          style={{ backgroundColor: hue }}
          role="slider"
          tabIndex={0}
          aria-label="Saturation and brightness"
          aria-valuetext={`Saturation ${Math.round(hsv.s * 100)}%, brightness ${Math.round(hsv.v * 100)}%`}
          onPointerDown={(event) => drag(event, "square")}
          onKeyDown={squareKey}
        >
          <span
            className="color-picker-knob"
            style={{
              left: `${(hsv.s * 100).toFixed(2)}%`,
              top: `${((1 - hsv.v) * 100).toFixed(2)}%`,
              background: mixed,
            }}
          />
        </div>
        <div
          className="color-picker-hue"
          role="slider"
          tabIndex={0}
          aria-label="Hue"
          aria-valuemin={0}
          aria-valuemax={360}
          aria-valuenow={Math.round(hsv.h)}
          onPointerDown={(event) => drag(event, "hue")}
          onKeyDown={hueKey}
        >
          <span
            className="color-picker-knob"
            style={{ left: `${((hsv.h / 360) * 100).toFixed(2)}%`, background: hue }}
          />
        </div>
        <div className="color-picker-row">
          <span className="color-picker-compare" title="Before · after">
            <i style={{ background: start }} />
            <i style={{ background: parsed ?? start }} />
          </span>
          <label className="color-picker-hex" data-wrong={wrong || undefined}>
            <span aria-hidden="true">#</span>
            <input
              ref={field}
              type="text"
              value={typed}
              aria-label="Hex colour, #rgb or #rrggbb"
              aria-invalid={parsed === null}
              spellCheck={false}
              autoComplete="off"
              maxLength={7}
              onChange={(event) => {
                const text = event.target.value.replace(/^#/, "").slice(0, 6);
                setTyped(text);
                setTried(false);
                const hex = parseHex(text);
                if (hex !== null) fromHex(hex);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  apply();
                }
              }}
            />
          </label>
          {Dropper && (
            <button
              type="button"
              className="color-picker-icon"
              title="Pick a colour from the screen"
              aria-label="Pick a colour from the screen"
              onClick={pick}
            >
              <Glyph name="pipette" size={14} />
            </button>
          )}
        </div>
        {wrong && (
          <div className="color-picker-error" role="alert">
            Use #rgb or #rrggbb, like #7af or #77aaff
          </div>
        )}
        <div className="color-picker-foot">
          <button type="button" className="color-picker-button" data-quiet="true" onClick={close}>
            Cancel
          </button>
          <button type="button" className="color-picker-button" onClick={apply}>
            Apply
          </button>
        </div>
      </div>
    </>
  );
}

/** Draws the page's colour picker while one is asked for. */
export function ColorPickerHost() {
  const ask = useSyncExternalStore(
    colorPicker.subscribe,
    () => colorPicker.getState().asked,
    () => null,
  );
  if (ask === null) return null;
  const close = () => {
    colorPicker.getState().close();
    ask.returnFocus?.();
  };
  // Keyed by what was asked, so a picker asked for again starts afresh.
  return createPortal(
    <Picker key={`${ask.label}:${ask.value}`} ask={ask} close={close} />,
    document.body,
  );
}
