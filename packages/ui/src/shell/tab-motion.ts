/**
 * How the row of tabs moves.
 *
 * Tabs that change places slide there from wherever they are on screen, so a second
 * change while a slide is under way carries on from where the tab is rather than
 * jumping. A new tab comes in, a renamed one settles, and a group's label and line
 * draw back before the group ends.
 *
 * The tab on show is marked by one shape that travels to the next tab on show: a pill
 * or an underline whose two ends are each held by a spring. The end facing the way it
 * goes is stiffer, so it leads, the other catches up, and the shape stretches and
 * settles. Its colour blends from the old tab's to the new one's over the way. With the
 * glow, speed blurs it and brightens it, and the label of each tab is shown bright where
 * the shape is.
 *
 * Changes the keyboard made, and every change when the system asks for less motion,
 * jump. The geometry of the row is read once for each change, never inside a frame.
 */

import type { TabHighlight } from "../core/types.ts";
import { TAB_EXIT_HELD_MS, TAB_FOLLOW_LAYOUT_MS, TAB_POINTER_RECENT_MS } from "../tuning.ts";

/** The curve most of the interface eases with. */
export const EASE = "cubic-bezier(0.23, 1, 0.32, 1)";

/** A spring: its stiffness and its damping ratio. */
export type Spring = readonly [stiffness: number, damping: number];

/** What one style of motion does. */
export interface MotionStyle {
  /** What marks the tab on show. */
  readonly shape: "pill" | "underline" | "glow";
  /** The springs of the end that leads and of the end that trails. */
  readonly lead: Spring;
  readonly trail: Spring;
  /** How tabs slide to new places: along a spring, or a curve for a time. */
  readonly slide: "spring" | { readonly duration: number; readonly easing: string };
  /** How a new tab comes in. */
  readonly enter: {
    readonly keyframes: readonly Keyframe[];
    readonly spring?: Spring;
    readonly duration?: number;
  };
  /** How a renamed label settles, if it does. */
  readonly renamed: readonly Keyframe[] | null;
}

/** The styles of motion, by the setting that chooses one. `instant` moves nothing. */
export const MOTION_STYLES: Readonly<Record<TabHighlight, MotionStyle | null>> = {
  instant: null,
  glide: {
    shape: "pill",
    lead: [600, 1],
    trail: [600, 1],
    slide: { duration: 220, easing: EASE },
    enter: {
      keyframes: [
        { opacity: 0, transform: "scale(0.94)" },
        { opacity: 1, transform: "none" },
      ],
      duration: 200,
    },
    renamed: [
      { opacity: 0, filter: "blur(2px)" },
      { opacity: 1, filter: "blur(0)" },
    ],
  },
  stretch: {
    shape: "underline",
    lead: [900, 1],
    trail: [300, 1],
    slide: { duration: 240, easing: EASE },
    enter: {
      keyframes: [
        { opacity: 0, transform: "translateY(-4px)" },
        { opacity: 1, transform: "none" },
      ],
      duration: 200,
    },
    renamed: [
      { opacity: 0, transform: "translateY(3px)" },
      { opacity: 1, transform: "none" },
    ],
  },
  spring: {
    shape: "pill",
    lead: [560, 0.82],
    trail: [560, 0.82],
    slide: "spring",
    enter: {
      keyframes: [
        { opacity: 0, transform: "scale(0.85)" },
        { opacity: 1, transform: "none" },
      ],
      spring: [600, 0.72],
    },
    renamed: null,
  },
  glow: {
    shape: "glow",
    lead: [820, 0.8],
    trail: [300, 0.9],
    slide: "spring",
    enter: {
      keyframes: [
        { opacity: 0, transform: "scale(0.85)", filter: "blur(2px)" },
        { opacity: 1, transform: "none", filter: "blur(0)" },
      ],
      spring: [600, 0.72],
    },
    renamed: [
      { opacity: 0, filter: "blur(2px)" },
      { opacity: 1, filter: "blur(0)" },
    ],
  },
};

/** A style by its name, or a plugin's as it is. */
function styleOf(style: TabHighlight | MotionStyle): MotionStyle | null {
  return typeof style === "string" ? MOTION_STYLES[style] : style;
}

/** The spring tabs slide along when a style slides along one. */
const SLIDE_SPRING: Spring = [520, 0.8];

/** A curve as CSS, how long it takes, and where along it a moment in it is. */
export interface Curve {
  readonly easing: string;
  readonly duration: number;
  /** How far along the move is, from 0 to 1 (and past 1 while it overshoots), at `u` of its time. */
  at(u: number): number;
}

/** How many steps of the simulation a sample of a spring's curve is taken every. */
const SPRING_SUBSTEPS = 4;
/** The step of the simulation, in seconds. */
const SPRING_DT = 1 / 240;
/** The longest a spring's curve is followed, in seconds. */
const SPRING_LIMIT_S = 1.2;

const curves = new Map<string, Curve>();

/**
 * A spring from rest at 0 to 1, as a curve: sampled into CSS's `linear()` where the
 * webview has it, else the interface's usual curve for as long. `linear` says whether it
 * has it.
 */
export function springCurve(spring: Spring, linear = true): Curve {
  const key = `${spring[0]}:${spring[1]}:${linear}`;
  const held = curves.get(key);
  if (held) return held;
  const [stiffness, damping] = spring;
  const friction = 2 * damping * Math.sqrt(stiffness);
  let x = 0;
  let v = 0;
  let t = 0;
  const points: number[] = [];
  while (t < SPRING_LIMIT_S) {
    for (let step = 0; step < SPRING_SUBSTEPS; step++) {
      const a = -stiffness * (x - 1) - friction * v;
      v += a * SPRING_DT;
      x += v * SPRING_DT;
      t += SPRING_DT;
    }
    points.push(Math.round(x * 10_000) / 10_000);
    if (t > 0.08 && Math.abs(x - 1) < 0.0015 && Math.abs(v) < 0.02) break;
  }
  points[points.length - 1] = 1;
  const all = [0, ...points];
  const curve: Curve = {
    easing: linear ? `linear(${all.join(", ")})` : EASE,
    duration: Math.round(t * 1000),
    at: (u) => {
      const along = Math.min(1, Math.max(0, u)) * (all.length - 1);
      const index = Math.floor(along);
      if (index >= all.length - 1) return 1;
      const from = all[index] ?? 0;
      const to = all[index + 1] ?? 1;
      return from + (to - from) * (along - index);
    },
  };
  curves.set(key, curve);
  return curve;
}

/** A cubic Bézier curve from (0, 0) to (1, 1), as a function of time. */
export function bezier(x1: number, y1: number, x2: number, y2: number): (u: number) => number {
  return (u) => {
    let t = u;
    for (let i = 0; i < 8; i++) {
      const x = 3 * (1 - t) ** 2 * t * x1 + 3 * (1 - t) * t * t * x2 + t ** 3 - u;
      const slope = 3 * (1 - t) ** 2 * x1 + 6 * (1 - t) * t * (x2 - x1) + 3 * t * t * (1 - x2);
      if (Math.abs(slope) < 1e-6) break;
      t = Math.min(1, Math.max(0, t - x / slope));
    }
    return 3 * (1 - t) ** 2 * t * y1 + 3 * (1 - t) * t * t * y2 + t ** 3;
  };
}

const easeAt = bezier(0.23, 1, 0.32, 1);

/** The curve tabs of a style slide along. */
export function slideCurve(style: MotionStyle, linear = true): Curve {
  if (style.slide === "spring") return springCurve(SLIDE_SPRING, linear);
  return { easing: style.slide.easing, duration: style.slide.duration, at: easeAt };
}

/** A slide under way: how far from its place a tab started, when, and along what. */
export interface Slide {
  readonly from: number;
  readonly started: number;
  readonly curve: Curve;
}

/** How far from its place a sliding tab is at `now`. */
export function slideOffset(slide: Slide, now: number): number {
  const u = (now - slide.started) / Math.max(1, slide.curve.duration);
  return u >= 1 ? 0 : slide.from * (1 - slide.curve.at(u));
}

/** A colour as its red, green and blue parts, from 0 to 255. */
export type Rgb = readonly [number, number, number];

/** A colour as CSS writes it (`#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(…)`), as its parts. */
export function rgbOf(text: string, fallback: Rgb): Rgb {
  const value = text.trim();
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(value)?.[1];
  if (hex) {
    const full = hex.length === 3 ? [...hex].map((digit) => digit + digit).join("") : hex;
    const n = Number.parseInt(full.slice(0, 6), 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  const parts = /^rgba?\(([^)]+)\)$/i
    .exec(value)?.[1]
    ?.split(/[\s,/]+/)
    .filter(Boolean);
  if (parts && parts.length >= 3) {
    const [r, g, b] = parts.map(Number);
    if ([r, g, b].every((part) => part !== undefined && Number.isFinite(part))) {
      return [r ?? 0, g ?? 0, b ?? 0];
    }
  }
  return fallback;
}

/** A colour part of the way from one to another. */
export function blend(from: Rgb, to: Rgb, part: number): Rgb {
  const p = Math.min(1, Math.max(0, part));
  return [
    from[0] + (to[0] - from[0]) * p,
    from[1] + (to[1] - from[1]) * p,
    from[2] + (to[2] - from[2]) * p,
  ];
}

/** The two ends of the mark and how fast each moves, in pixels and pixels a second. */
export interface Ends {
  left: number;
  right: number;
  vLeft: number;
  vRight: number;
}

/** A spring as the simulation uses it: its stiffness and its friction. */
interface Held {
  readonly k: number;
  readonly c: number;
}

function held([stiffness, damping]: Spring): Held {
  return { k: stiffness, c: 2 * damping * Math.sqrt(stiffness) };
}

/** How many smaller steps one frame of the mark's springs is taken in. */
const MARK_SUBSTEPS = 4;
/** The longest frame the springs take at once, in seconds: a page that was away does not leap. */
const MARK_FRAME_MAX_S = 0.032;

/**
 * Moves the mark's ends one frame towards their targets, each on its own spring.
 * Returns whether they have come to rest there.
 */
export function stepEnds(
  ends: Ends,
  target: { readonly left: number; readonly right: number },
  springs: { readonly left: Spring; readonly right: Spring },
  seconds: number,
): boolean {
  const left = held(springs.left);
  const right = held(springs.right);
  const h = Math.min(MARK_FRAME_MAX_S, Math.max(0, seconds)) / MARK_SUBSTEPS;
  for (let i = 0; i < MARK_SUBSTEPS; i++) {
    ends.vLeft += (-left.k * (ends.left - target.left) - left.c * ends.vLeft) * h;
    ends.left += ends.vLeft * h;
    ends.vRight += (-right.k * (ends.right - target.right) - right.c * ends.vRight) * h;
    ends.right += ends.vRight * h;
  }
  return (
    Math.abs(ends.left - target.left) < 0.3 &&
    Math.abs(ends.right - target.right) < 0.3 &&
    Math.abs(ends.vLeft) < 3 &&
    Math.abs(ends.vRight) < 3
  );
}

/**
 * The springs of the two ends for a move: the end facing the way the mark goes leads.
 * Equal springs move the mark rigidly; a stiffer lead stretches it.
 */
export function springsFor(
  style: MotionStyle,
  rightwards: boolean,
): { readonly left: Spring; readonly right: Spring } {
  return rightwards
    ? { left: style.trail, right: style.lead }
    : { left: style.lead, right: style.trail };
}

/** How far the underline reaches in from each side of its tab. */
const UNDERLINE_INSET_PX = 10;
/** The narrowest the mark gets while it travels. */
const MARK_MIN_PX = 8;
/** The speed, in pixels a second, at which the glow is at its strongest. */
const GLOW_FULL_SPEED = 1_600;
/** The speed past which the glow starts to blur, and the blur at its strongest. */
const GLOW_BLUR_FROM = 40;
const GLOW_BLUR_MAX_PX = 3;

/** The colour the mark takes for a tab that says none. */
const FALLBACK_TINT: Rgb = [127, 227, 184];

/** Whether the webview eases with `linear()`. */
function hasLinear(): boolean {
  return typeof CSS !== "undefined" && CSS.supports("animation-timing-function", "linear(0, 1)");
}

function reducedMotion(): boolean {
  return (
    typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** What the row looked like the last time it was measured, for one thing in it. */
interface Measured {
  readonly element: HTMLElement;
  readonly x: number;
}

/** A tab's bright label and where it is in the row, for the glow to reveal. */
interface LitBox {
  readonly lit: HTMLElement;
  readonly x: number;
  readonly width: number;
}

/** How the row of tabs is to be brought up to date with what it now shows. */
export interface SyncOptions {
  /** One of the window's own styles, by name, or a plugin's. */
  readonly style: TabHighlight | MotionStyle;
  /** The tab on show. */
  readonly active: string | null;
  /** Whether this change may move: the pointer made it, and the system allows motion. */
  readonly animate: boolean;
  /** The tab whose new name has just been kept, to settle. */
  readonly renamed: string | null;
}

/**
 * Moves one row of tabs: `list` is the element the tabs and the groups' labels are in,
 * each named by `data-tab-id`, with the mark first among them.
 */
export class TabMotion {
  readonly #list: HTMLElement;
  readonly #mark: HTMLElement;
  #places = new Map<string, number>();
  #slides = new Map<string, Slide>();
  #exits: Animation[] = [];
  #style: TabHighlight | MotionStyle | null = null;
  #lastPointer = Number.NEGATIVE_INFINITY;
  // The mark.
  #ends: Ends | null = null;
  #target = { left: 0, right: 0 };
  #springs: { left: Spring; right: Spring } | null = null;
  #from: Rgb = FALLBACK_TINT;
  #to: Rgb = FALLBACK_TINT;
  #colour: Rgb = FALLBACK_TINT;
  #middleFrom = 0;
  #middleTo = 0;
  #frame = 0;
  #lastFrame = 0;
  #lit: LitBox[] = [];
  #follow = 0;
  #options: SyncOptions | null = null;

  constructor(list: HTMLElement, mark: HTMLElement) {
    this.#list = list;
    this.#mark = mark;
  }

  /** Notes that the pointer pressed something among the tabs just now. */
  pointer(): void {
    this.#lastPointer = performance.now();
  }

  /** Whether a change now would be the pointer's doing. */
  pointerRecent(): boolean {
    return performance.now() - this.#lastPointer < TAB_POINTER_RECENT_MS && !reducedMotion();
  }

  /** Presses the mark down with the tab under it, until the pointer lets go. */
  press(): void {
    const mark = this.#mark;
    mark.classList.add("pressed");
    const up = () => mark.classList.remove("pressed");
    window.addEventListener("pointerup", up, { once: true });
    window.addEventListener("pointercancel", up, { once: true });
  }

  #items(): HTMLElement[] {
    return [...this.#list.querySelectorAll<HTMLElement>(":scope > [data-tab-id]")];
  }

  /** Brings the row up to date with what it shows now. Called after each render of it. */
  sync(options: SyncOptions): void {
    this.#options = options;
    for (const exit of this.#exits) exit.cancel();
    this.#exits = [];
    const style = styleOf(options.style);
    if (this.#style !== options.style) this.#restyle(options.style);
    const items = this.#items();
    const now = performance.now();
    const animate = options.animate && style !== null && !reducedMotion();

    // Where each was on screen a moment ago: its old place, and any slide under way.
    const seen = new Map<string, number>();
    for (const [id, x] of this.#places) {
      const slide = this.#slides.get(id);
      seen.set(id, x + (slide ? slideOffset(slide, now) : 0));
    }
    for (const item of items) {
      for (const running of item.getAnimations()) {
        if (running.id === "tab-slide") running.cancel();
      }
    }
    this.#slides = new Map();
    const measured: Measured[] = items.map((element) => ({ element, x: element.offsetLeft }));
    if (animate && style && this.#places.size > 0) {
      const curve = slideCurve(style, hasLinear());
      const enter = style.enter.spring ? springCurve(style.enter.spring, hasLinear()) : null;
      for (const { element, x } of measured) {
        const id = element.dataset["tabId"] ?? "";
        const was = seen.get(id);
        if (was === undefined) {
          element.animate([...style.enter.keyframes], {
            duration: enter?.duration ?? style.enter.duration ?? 200,
            easing: enter?.easing ?? EASE,
          });
          continue;
        }
        const from = was - x;
        if (Math.abs(from) <= 0.5) continue;
        const slide = element.animate(
          [{ transform: `translateX(${from}px)` }, { transform: "none" }],
          { duration: curve.duration, easing: curve.easing },
        );
        slide.id = "tab-slide";
        this.#slides.set(id, { from, started: now, curve });
      }
    }
    if (animate && style?.renamed && options.renamed !== null) {
      const renamed = items.find((item) => item.dataset["tabId"] === options.renamed);
      for (const title of renamed?.querySelectorAll(".tab-title") ?? []) {
        title.animate([...style.renamed], { duration: 180, easing: EASE });
      }
    }
    this.#places = new Map(measured.map(({ element, x }) => [element.dataset["tabId"] ?? "", x]));
    this.#placeMark(style, animate, items);
  }

  /** Keeps the mark with tabs the style sheet is moving, for `ms`. */
  followLayout(ms = TAB_FOLLOW_LAYOUT_MS): void {
    if (reducedMotion()) return;
    const end = performance.now() + ms;
    if (this.#follow !== 0) cancelAnimationFrame(this.#follow);
    const step = () => {
      const options = this.#options;
      if (!options) return;
      const items = this.#items();
      this.#places = new Map(items.map((item) => [item.dataset["tabId"] ?? "", item.offsetLeft]));
      // A mark that is travelling goes on to the new place; one at rest stays on its tab.
      this.#placeMark(styleOf(options.style), this.#frame !== 0, items);
      this.#follow = performance.now() < end ? requestAnimationFrame(step) : 0;
    };
    this.#follow = requestAnimationFrame(step);
  }

  /** Clears what the last style painted, when another is chosen. */
  #restyle(style: TabHighlight | MotionStyle): void {
    this.#style = style;
    if (this.#frame !== 0) cancelAnimationFrame(this.#frame);
    this.#frame = 0;
    this.#ends = null;
    const mark = this.#mark;
    for (const name of [
      "transform",
      "width",
      "filter",
      "top",
      "height",
      "opacity",
      "--mark-speed",
      "--mark-rgb",
    ]) {
      mark.style.removeProperty(name);
    }
    mark.classList.remove("pressed", "tinted");
    for (const lit of this.#list.querySelectorAll<HTMLElement>(".tab-lit")) {
      lit.style.clipPath = "inset(0 100% 0 0)";
    }
  }

  #placeMark(style: MotionStyle | null, animate: boolean, items: readonly HTMLElement[]): void {
    const list = this.#list;
    const mark = this.#mark;
    if (!style) {
      delete list.dataset["moving"];
      mark.style.opacity = "0";
      return;
    }
    list.dataset["moving"] = "true";
    const active = this.#options?.active ?? null;
    const tab = items.find((item) => item.dataset["tabId"] === active && item.matches(".tab"));
    if (!tab) {
      mark.style.opacity = "0";
      return;
    }
    mark.style.opacity = "";
    if (style.shape === "underline") {
      mark.style.top = `${tab.offsetTop + tab.offsetHeight - 7}px`;
      mark.style.removeProperty("height");
    } else {
      mark.style.top = `${tab.offsetTop}px`;
      mark.style.height = `${tab.offsetHeight}px`;
    }
    this.#lit = items.flatMap((item): LitBox[] => {
      const lit = item.querySelector<HTMLElement>(".tab-lit");
      const label = lit?.parentElement;
      if (!lit || !label) return [];
      return [{ lit, x: item.offsetLeft + label.offsetLeft, width: lit.offsetWidth }];
    });
    const colour = rgbOf(getComputedStyle(tab).getPropertyValue("--tab-tint"), FALLBACK_TINT);
    mark.classList.toggle("tinted", tab.dataset["tinted"] === "true");
    const inset = style.shape === "underline" ? UNDERLINE_INSET_PX : 0;
    const target = {
      left: tab.offsetLeft + inset,
      right: tab.offsetLeft + Math.max(inset * 2 + 12, tab.offsetWidth) - inset,
    };
    const ends = this.#ends;
    if (!ends || !animate) {
      this.#jump(target, colour);
      return;
    }
    if (target.left === this.#target.left && target.right === this.#target.right) {
      if (this.#frame === 0) this.#paint();
      return;
    }
    const rightwards = target.left > this.#target.left;
    this.#springs = springsFor(style, rightwards);
    this.#from = this.#colour;
    this.#to = colour;
    this.#middleFrom = (ends.left + ends.right) / 2;
    this.#middleTo = (target.left + target.right) / 2;
    this.#target = target;
    if (this.#frame !== 0) return;
    // The first frame is the next one: nothing moves before the change is on screen.
    this.#lastFrame = performance.now();
    this.#frame = requestAnimationFrame(this.#tick);
  }

  readonly #tick = (now: number): void => {
    const ends = this.#ends;
    const springs = this.#springs;
    if (!ends || !springs) {
      this.#frame = 0;
      return;
    }
    const seconds = (now - this.#lastFrame) / 1000;
    this.#lastFrame = now;
    const still = stepEnds(ends, this.#target, springs, seconds);
    if (still) {
      this.#jump(this.#target, this.#to);
      return;
    }
    this.#paint();
    this.#frame = requestAnimationFrame(this.#tick);
  };

  #jump(target: { left: number; right: number }, colour: Rgb): void {
    if (this.#frame !== 0) cancelAnimationFrame(this.#frame);
    this.#frame = 0;
    this.#target = target;
    this.#ends = { left: target.left, right: target.right, vLeft: 0, vRight: 0 };
    this.#from = colour;
    this.#to = colour;
    this.#colour = colour;
    this.#middleFrom = (target.left + target.right) / 2;
    this.#middleTo = this.#middleFrom;
    this.#paint();
  }

  #paint(): void {
    const ends = this.#ends;
    const style = this.#options ? styleOf(this.#options.style) : null;
    if (!ends || !style) return;
    const mark = this.#mark;
    const speed = Math.max(Math.abs(ends.vLeft), Math.abs(ends.vRight));
    const span = this.#middleTo - this.#middleFrom;
    const along = Math.abs(span) < 1 ? 1 : ((ends.left + ends.right) / 2 - this.#middleFrom) / span;
    this.#colour = blend(this.#from, this.#to, along);
    const width = Math.max(MARK_MIN_PX, ends.right - ends.left);
    if (style.shape === "underline") {
      // Scaled, not resized: the underline is drawn by the compositor alone.
      mark.style.transform = `translateX(${ends.left}px) scaleX(${Math.max(1, width)})`;
    } else {
      mark.style.transform = `translateX(${ends.left}px) scale(var(--mark-press))`;
      mark.style.width = `${width}px`;
    }
    if (style.shape === "glow") {
      const strength = Math.min(1, speed / GLOW_FULL_SPEED);
      mark.style.filter =
        speed > GLOW_BLUR_FROM
          ? `blur(${Math.min(GLOW_BLUR_MAX_PX, speed / 900).toFixed(2)}px)`
          : "";
      mark.style.setProperty("--mark-speed", strength.toFixed(3));
    }
    mark.style.setProperty("--mark-rgb", this.#colour.map(Math.round).join(" "));
    const pad = style.shape === "underline" ? UNDERLINE_INSET_PX : 0;
    for (const box of this.#lit) {
      const left = ends.left - pad - box.x;
      const right = ends.right + pad - box.x;
      box.lit.style.clipPath = `inset(0 ${Math.max(0, box.width - right).toFixed(1)}px 0 ${Math.max(0, left).toFixed(1)}px)`;
    }
  }

  /**
   * Draws a group's label and line back before the group ends. Resolves once they
   * have, at once when the system asks for less motion.
   */
  groupExit(group: string): Promise<void> {
    const label = this.#list.querySelector<HTMLElement>(`[data-tab-id="L${group}"]`);
    if (!label || reducedMotion()) return Promise.resolve();
    const exits: Animation[] = [
      label.animate(
        [
          { opacity: 1, transform: "none", transformOrigin: "left center" },
          { opacity: 0, transform: "scale(0.6)", transformOrigin: "left center" },
        ],
        { duration: 140, easing: EASE, fill: "forwards" },
      ),
    ];
    for (const line of this.#list.querySelectorAll<HTMLElement>(
      `[data-group="${group}"] > .tab-group-line, [data-tab-id="L${group}"] > .tab-group-line`,
    )) {
      exits.push(
        line.animate(
          [
            { transform: "none", transformOrigin: "right center" },
            { transform: "scaleX(0)", transformOrigin: "right center" },
          ],
          { duration: 170, easing: EASE, fill: "forwards" },
        ),
      );
    }
    return this.#held(exits);
  }

  /** Fades a group's label and tabs out before they close or leave. */
  groupFade(group: string): Promise<void> {
    const parts = [
      ...this.#list.querySelectorAll<HTMLElement>(
        `[data-tab-id="L${group}"], .tab[data-group="${group}"]`,
      ),
    ];
    if (parts.length === 0 || reducedMotion()) return Promise.resolve();
    return this.#held(
      parts.map((part) =>
        part.animate(
          [
            { opacity: 1, transform: "none" },
            { opacity: 0, transform: "scale(0.96)" },
          ],
          { duration: 130, easing: EASE, fill: "forwards" },
        ),
      ),
    );
  }

  /**
   * Keeps animations that leave their elements as they end until the row changes,
   * since the elements are used again; and only for so long, should it not.
   */
  #held(exits: Animation[]): Promise<void> {
    this.#exits.push(...exits);
    const done = Promise.all(exits.map((exit) => exit.finished)).then(
      () => undefined,
      () => undefined,
    );
    void done.then(() =>
      window.setTimeout(() => {
        for (const exit of exits) exit.cancel();
      }, TAB_EXIT_HELD_MS),
    );
    return done;
  }

  dispose(): void {
    if (this.#frame !== 0) cancelAnimationFrame(this.#frame);
    if (this.#follow !== 0) cancelAnimationFrame(this.#follow);
    for (const exit of this.#exits) exit.cancel();
  }
}
