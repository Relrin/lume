/**
 * A page written down as what it is made of, so that two runs can be compared without
 * comparing pictures.
 *
 * A picture of a window differs from machine to machine in ways nobody cares about: how
 * a letter's edge is softened, how wide a word comes out. What is written down here is
 * what a change to the page's code changes: which parts there are and in what order,
 * what each is called and what state it says it is in, and, for the parts whose size
 * comes from the design's own numbers and not from text, where each is to the pixel.
 *
 * A page is read through a few properties of its elements and one function that
 * measures, so a description of a page can be read the same way as a page.
 */

import { roleOf, type AuditNode } from "./audit.ts";

/** One part of a page. A part with nothing to say of some kind leaves that kind out. */
export interface Part {
  /** What the part is to a screen reader. */
  readonly role?: string;
  /** Which of the page's measured boxes it is, by its class. */
  readonly box?: string;
  readonly name?: string;
  /** What it says of itself: selected, pressed, expanded, focused and the like. */
  readonly state?: Readonly<Record<string, string>>;
  /** Left, top, width and height in whole pixels, from the page's top left corner. */
  readonly at?: readonly [number, number, number, number];
  readonly parts?: readonly Part[];
}

/** A rectangle as a page measures one. */
export interface Measured {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/**
 * The classes of the parts that are measured. Each is sized by the layout and the
 * design's lengths alone: a bar of a fixed height, a pane in its share of the window, a
 * panel of the width the layout gives it. A part sized by its text is left out, since
 * text is as wide as the machine's fonts make it.
 */
export const MEASURED_BOXES: readonly string[] = [
  "shell",
  "top-bar",
  "shell-body",
  "shell-centre",
  "tab-area",
  "pane-card",
  "pane-header",
  "terminal",
  "divider",
  "dock-area",
  "dock-resize",
  "panel",
  "panel-header",
  "panel-body",
  "settings-window",
  "settings-titlebar",
  "settings-side",
  "settings-main",
  "settings-head",
];

/** What a part says of itself, by the attribute it says it in. */
const STATES: readonly (readonly [string, string])[] = [
  ["aria-selected", "selected"],
  ["aria-pressed", "pressed"],
  ["aria-checked", "checked"],
  ["aria-expanded", "expanded"],
  ["aria-current", "current"],
  ["aria-disabled", "disabled"],
  ["aria-orientation", "orientation"],
  ["aria-modal", "modal"],
  ["data-focused", "focused"],
  ["data-floating", "floating"],
  ["data-area", "area"],
  ["data-edge", "edge"],
];

/** The roles of the elements that mark out a page without claiming a role. */
const LANDMARKS: Readonly<Record<string, string>> = {
  NAV: "navigation",
  MAIN: "main",
  ASIDE: "complementary",
  H1: "heading",
  H2: "heading",
  H3: "heading",
  H4: "heading",
};

/** The roles that are named by what is written in them. */
const NAMED_BY_CONTENT = new Set([
  "button",
  "link",
  "tab",
  "radio",
  "menuitem",
  "menuitemradio",
  "option",
  "heading",
  "status",
  "alert",
]);

/** The longest name written down. A longer one is cut: it is there to be recognised. */
const NAME_MAX_CHARS = 80;

function structuralRole(node: AuditNode): string | null {
  const role = roleOf(node);
  if (role !== null) return role;
  if (node.tagName === "SECTION") {
    return node.getAttribute("aria-label") === null ? null : "region";
  }
  return LANDMARKS[node.tagName] ?? null;
}

function measuredBox(node: AuditNode): string | null {
  const classes = (node.getAttribute("class") ?? "").split(/\s+/);
  return MEASURED_BOXES.find((box) => classes.includes(box)) ?? null;
}

function nameOf(node: AuditNode, role: string | null): string | undefined {
  const label = (node.getAttribute("aria-label") ?? "").trim();
  if (label !== "") return label.slice(0, NAME_MAX_CHARS);
  if (role === null || !NAMED_BY_CONTENT.has(role)) return undefined;
  const text = (node.textContent ?? "").replace(/\s+/g, " ").trim();
  return text === "" ? undefined : text.slice(0, NAME_MAX_CHARS);
}

/**
 * Writes down the page under `root`. `measure` says where an element is; `origin` is
 * where the page's top left corner is in the same terms.
 *
 * An element that has no role and is no measured box is not a part itself: its parts
 * count as parts of whatever it is in. What is hidden from a screen reader, or hidden
 * altogether, is left out with everything in it.
 */
export function describePage(
  root: AuditNode,
  measure: (node: AuditNode) => Measured,
  origin: { readonly x: number; readonly y: number },
): Part {
  const partsIn = (node: AuditNode): Part[] => {
    const parts: Part[] = [];
    for (let index = 0; index < node.children.length; index++) {
      const child = node.children[index];
      if (child) parts.push(...describe(child));
    }
    return parts;
  };
  const describe = (node: AuditNode): Part[] => {
    if (node.getAttribute("aria-hidden") === "true" || node.getAttribute("hidden") !== null) {
      return [];
    }
    const role = structuralRole(node);
    const box = measuredBox(node);
    const inner = partsIn(node);
    if (role === null && box === null) return inner;

    const state: Record<string, string> = {};
    for (const [attribute, called] of STATES) {
      const value = node.getAttribute(attribute);
      if (value !== null) state[called] = value;
    }
    const name = nameOf(node, role);
    let at: Part["at"];
    if (box !== null) {
      const rect = measure(node);
      at = [
        Math.round(rect.x - origin.x),
        Math.round(rect.y - origin.y),
        Math.round(rect.width),
        Math.round(rect.height),
      ];
    }
    return [
      {
        ...(role === null ? {} : { role }),
        ...(box === null ? {} : { box }),
        ...(name === undefined ? {} : { name }),
        ...(Object.keys(state).length === 0 ? {} : { state }),
        ...(at === undefined ? {} : { at }),
        ...(inner.length === 0 ? {} : { parts: inner }),
      },
    ];
  };
  const described = describe(root);
  // A root that is no part itself still gives the page one top.
  return described.length === 1 && described[0] ? described[0] : { parts: described };
}

/** A part in a few words, for saying where two pages differ. */
function label(part: Part): string {
  const what = part.role ?? part.box ?? "part";
  return part.name === undefined ? what : `${what} "${part.name}"`;
}

/**
 * Where two pages differ, each difference as a sentence that says where. Nothing
 * different is an empty list. A measured box may be `tolerance` pixels off along any
 * side: a share of a window does not always come out on a whole pixel.
 */
export function differences(expected: Part, found: Part, tolerance: number): string[] {
  const problems: string[] = [];
  const compare = (want: Part, have: Part, path: string): void => {
    const here = `${path}${label(want)}`;
    if (want.role !== have.role || want.box !== have.box) {
      problems.push(`${path}${label(have)} is where ${label(want)} was`);
      return;
    }
    if (want.name !== have.name) {
      problems.push(`${here} is now called "${have.name ?? ""}"`);
    }
    const states = new Set([...Object.keys(want.state ?? {}), ...Object.keys(have.state ?? {})]);
    for (const state of states) {
      const before = want.state?.[state];
      const now = have.state?.[state];
      if (before !== now) {
        problems.push(`${here}: ${state} was ${before ?? "not said"} and is ${now ?? "not said"}`);
      }
    }
    if (want.at && have.at) {
      const off = want.at.some((value, index) => {
        return Math.abs(value - (have.at?.[index] ?? Number.NaN)) > tolerance;
      });
      if (off) problems.push(`${here} was at ${want.at.join(",")} and is at ${have.at.join(",")}`);
    } else if (want.at || have.at) {
      problems.push(`${here} is ${have.at ? "now" : "no longer"} measured`);
    }
    const wanted = want.parts ?? [];
    const had = have.parts ?? [];
    if (wanted.length !== had.length) {
      problems.push(`${here} had ${wanted.length} parts and has ${had.length}`);
      return;
    }
    wanted.forEach((part, index) => {
      const other = had[index];
      if (other) compare(part, other, `${here} › `);
    });
  };
  compare(expected, found, "");
  return problems;
}

/**
 * A colour as a page reports one, written one way: `#rrggbb`, or `#rrggbbaa` when it is
 * not opaque. A colour worked out from two others is reported differently by different
 * webviews, to more digits than anyone chose, and is written as `mixed`.
 */
export function plainColor(reported: string): string {
  const text = reported.trim().toLowerCase();
  const hex = /^#([0-9a-f]{6})([0-9a-f]{2})?$/.exec(text);
  if (hex) return hex[2] === undefined || hex[2] === "ff" ? `#${hex[1]}` : text;
  const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/.exec(text);
  if (short) return `#${short[1]}${short[1]}${short[2]}${short[2]}${short[3]}${short[3]}`;
  const rgb = /^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)(?:\s*[,/]\s*([\d.]+%?))?\s*\)$/.exec(text);
  if (!rgb) return "mixed";
  const channel = (value: string | undefined): string => {
    return Math.min(255, Number(value)).toString(16).padStart(2, "0");
  };
  let alpha = 1;
  if (rgb[4] !== undefined) {
    alpha = rgb[4].endsWith("%") ? Number(rgb[4].slice(0, -1)) / 100 : Number(rgb[4]);
  }
  const opaque = `#${channel(rgb[1])}${channel(rgb[2])}${channel(rgb[3])}`;
  if (alpha >= 1) return opaque;
  return `${opaque}${Math.round(Math.max(0, alpha) * 255)
    .toString(16)
    .padStart(2, "0")}`;
}

/** The widest a line of a written record may be before a list of numbers is broken up. */
const RECORD_LINE_MAX = 100;

/**
 * A record as the text of a file: JSON, indented, with a short list of numbers on one
 * line. The same record always comes out as the same text, so a file that has not
 * changed does not look changed.
 */
export function recordText(value: unknown): string {
  const write = (item: unknown, indent: string): string => {
    if (Array.isArray(item)) {
      if (item.length === 0) return "[]";
      const inner = `${indent}  `;
      if (item.every((entry) => typeof entry === "number")) {
        const line = `[${item.join(", ")}]`;
        if (indent.length + line.length < RECORD_LINE_MAX) return line;
      }
      const entries = item.map((entry) => `${inner}${write(entry, inner)}`);
      return `[\n${entries.join(",\n")}\n${indent}]`;
    }
    if (typeof item === "object" && item !== null) {
      const entries = Object.entries(item as Record<string, unknown>).filter(
        ([, entry]) => entry !== undefined,
      );
      if (entries.length === 0) return "{}";
      const inner = `${indent}  `;
      const lines = entries.map(
        ([key, entry]) => `${inner}${JSON.stringify(key)}: ${write(entry, inner)}`,
      );
      return `{\n${lines.join(",\n")}\n${indent}}`;
    }
    return JSON.stringify(item ?? null);
  };
  return `${write(value, "")}\n`;
}
