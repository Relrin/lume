/**
 * The settings as pages of rows, worked out from what the core sends.
 *
 * Nothing about a setting is written down here. The configuration's schema says which
 * settings a person is shown, on which page and under which heading, with what kind of
 * control and which choices; the configuration in use says what each is set to and
 * where that came from. This puts the two together, with the lists only the machine
 * knows: its themes, fonts, shells, workspaces and profiles.
 *
 * A page is plain data, so a page that does not come from the schema can be drawn by
 * the same window. Nothing here touches the page, so all of it is tested with plain
 * objects.
 */

import type { ComponentType } from "react";

import { describeProblem } from "../config/apply.ts";
import type { ConfigView, Profile, ShellEntry, ThemeEntry } from "../core/types.ts";
import { profileChoices } from "../palette/choices.ts";

/** What a setting is set to. */
export type SettingValue = string | number | boolean;

/** The kinds of control a row has. */
export type SettingControl = "switch" | "select" | "segmented" | "text";

/** The lists a setting's choices may be taken from. */
export type SettingList = "themes" | "fonts" | "shells" | "workspaces" | "profiles";

/** What the schema says of a setting a person is shown: the keyword `x-lume-settings`. */
export interface SettingAnnotation {
  readonly page: string;
  readonly group: string;
  readonly order: number;
  readonly label: string;
  readonly help?: string;
  readonly control: SettingControl;
  readonly options?: readonly { readonly value: SettingValue; readonly label: string }[];
  readonly list?: SettingList;
  /** What the choice that takes the setting out of the file is called. */
  readonly unset?: string;
  /** The design token that says what the setting comes to while it is left out. */
  readonly token?: string;
  readonly unit?: string;
  /** The row is shown only while another setting is, or is not, in the file. */
  readonly when?: { readonly path: string; readonly is: "unset" | "set" };
}

/** What the schema says of a page of settings: one entry of the keyword `x-lume-pages`. */
export interface PageAnnotation {
  readonly id: string;
  readonly label: string;
  readonly icon: string;
  readonly group: string;
  readonly blurb: string;
  /** The page's headings, in the order they are shown. */
  readonly groups: readonly string[];
}

/** One thing a setting can be set to. `null` takes the setting out of the file. */
export interface SettingChoice {
  readonly value: SettingValue | null;
  readonly label: string;
}

/** The layer of the configuration a setting's value came from. */
export type SettingSource = "default" | "user" | "workspace" | "profile";

/** One setting, as a row shows it. */
export interface SettingRow {
  /** The setting's path, such as `font.size`. */
  readonly path: string;
  readonly label: string;
  readonly help?: string;
  readonly control: SettingControl;
  /** What it may be set to. Empty for a switch and for text. */
  readonly choices: readonly SettingChoice[];
  /** What it is set to, or `null` while it is left out. */
  readonly value: SettingValue | null;
  /** What it comes to: its value, or what stands in while it is left out. */
  readonly effective: SettingValue | null;
  /** What the choice that leaves the setting out is called, if there is one. */
  readonly unsetLabel?: string;
  readonly source: SettingSource;
  /** Whether a file other than the user's sets it, so that writing it changes nothing. */
  readonly overridden: boolean;
  /** What is wrong with it in the user's files, if anything is. */
  readonly problem?: string;
  /** Whether the row is shown, given what other settings are. */
  readonly shown: boolean;
}

/** The rows under one heading. */
export interface SettingsGroup {
  readonly label: string;
  readonly rows: readonly SettingRow[];
}

/** One page of the settings window. */
export interface SettingsPage {
  readonly id: string;
  readonly label: string;
  /** The name of the page's icon. */
  readonly icon: string;
  /** The heading the page is listed under. */
  readonly group: string;
  /** A version, or another word or two shown beside the page's name. */
  readonly meta?: string;
  readonly blurb: string;
  readonly groups: readonly SettingsGroup[];
  /** What the page shows in the column beside its rows, if it shows anything there. */
  readonly preview?: ComponentType;
}

/** What only the machine knows that settings are chosen from. */
export interface SettingsLists {
  readonly themes: readonly ThemeEntry[];
  readonly fonts: readonly string[];
  readonly shells: readonly ShellEntry[];
  readonly workspaces: readonly string[];
  readonly profiles: readonly Profile[];
}

/** No lists: what a window has before the core has sent any. */
export const NO_LISTS: SettingsLists = {
  themes: [],
  fonts: [],
  shells: [],
  workspaces: [],
  profiles: [],
};

/** A change to the user's file that a row asks for. */
export type SettingWrite =
  | { readonly op: "set"; readonly path: string; readonly value: SettingValue }
  | { readonly op: "unset"; readonly path: string };

/** The rows of every page that match what was typed into the search field. */
export interface SettingsSearch {
  readonly groups: readonly {
    /** Where the rows are: the pages' heading, the page and the rows' heading. */
    readonly label: string;
    /** The page the rows are on. */
    readonly page: string;
    readonly rows: readonly SettingRow[];
  }[];
  /** How many rows match. */
  readonly count: number;
}

/** What separates the parts of a place in the settings, as a person reads it. */
const PLACE_SEPARATOR = " › ";

type Json = Readonly<Record<string, unknown>>;

function isObject(value: unknown): value is Json {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isValue(value: unknown): value is SettingValue {
  return typeof value === "string" || typeof value === "number" || typeof value === "boolean";
}

function text(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

const CONTROLS: readonly SettingControl[] = ["switch", "select", "segmented", "text"];
const LISTS: readonly SettingList[] = ["themes", "fonts", "shells", "workspaces", "profiles"];

/** What the schema says of a setting, if it says it in the form this reads. */
function annotationOf(raw: unknown): SettingAnnotation | undefined {
  if (!isObject(raw)) return undefined;
  const { page, group, order, label, control } = raw;
  if (typeof page !== "string" || typeof group !== "string" || typeof label !== "string") {
    return undefined;
  }
  if (typeof order !== "number" || !CONTROLS.includes(control as SettingControl)) return undefined;
  const options = Array.isArray(raw["options"])
    ? raw["options"].flatMap((option: unknown) =>
        isObject(option) && isValue(option["value"]) && typeof option["label"] === "string"
          ? [{ value: option["value"], label: option["label"] }]
          : [],
      )
    : undefined;
  const when = raw["when"];
  const help = text(raw["help"]);
  const unset = text(raw["unset"]);
  const token = text(raw["token"]);
  const unit = text(raw["unit"]);
  return {
    page,
    group,
    order,
    label,
    control: control as SettingControl,
    ...(help === undefined ? {} : { help }),
    ...(options === undefined ? {} : { options }),
    ...(LISTS.includes(raw["list"] as SettingList) ? { list: raw["list"] as SettingList } : {}),
    ...(unset === undefined ? {} : { unset }),
    ...(token === undefined ? {} : { token }),
    ...(unit === undefined ? {} : { unit }),
    ...(isObject(when) &&
    typeof when["path"] === "string" &&
    (when["is"] === "unset" || when["is"] === "set")
      ? { when: { path: when["path"], is: when["is"] } }
      : {}),
  };
}

function pageOf(raw: unknown): PageAnnotation | undefined {
  if (!isObject(raw)) return undefined;
  const { id, label, icon, group, blurb, groups } = raw;
  if (typeof id !== "string" || typeof label !== "string" || typeof group !== "string") {
    return undefined;
  }
  return {
    id,
    label,
    group,
    icon: typeof icon === "string" ? icon : id,
    blurb: typeof blurb === "string" ? blurb : "",
    groups: Array.isArray(groups) ? groups.filter((name) => typeof name === "string") : [],
  };
}

/** A setting the schema shows to a person: where it is, and what the schema says of it. */
interface Found {
  readonly path: string;
  readonly schema: Json;
  readonly annotation: SettingAnnotation;
}

/** Every setting of a schema that a person is shown, wherever in it the setting is. */
function annotated(schema: Json, prefix: string, found: Found[]): Found[] {
  const properties = schema["properties"];
  if (!isObject(properties)) return found;
  for (const [key, child] of Object.entries(properties)) {
    if (!isObject(child)) continue;
    const path = prefix === "" ? key : `${prefix}.${key}`;
    const annotation = annotationOf(child["x-lume-settings"]);
    if (annotation) found.push({ path, schema: child, annotation });
    else annotated(child, path, found);
  }
  return found;
}

/** What a setting is set to in a configuration, or `null` when it is left out. */
function valueAt(config: unknown, path: string): SettingValue | null {
  let at: unknown = config;
  for (const part of path.split(".")) {
    if (!isObject(at)) return null;
    at = at[part];
  }
  return isValue(at) ? at : null;
}

/** What a value reads as with its unit: `10 px`, or `80%`. */
function withUnit(value: SettingValue, unit: string | undefined): string {
  if (unit === undefined || unit === "") return String(value);
  return unit === "%" ? `${value}%` : `${value} ${unit}`;
}

/** What a word of the configuration reads as: `close-on-exit` becomes `Close on exit`. */
function worded(value: SettingValue): string {
  const words = String(value).replace(/[-_]+/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** The values a schema allows a setting, when it names them one by one. */
function enumerated(schema: Json): SettingValue[] {
  const named = (candidates: unknown): SettingValue[] =>
    Array.isArray(candidates)
      ? candidates.flatMap((candidate: unknown) =>
          isObject(candidate) && isValue(candidate["const"]) ? [candidate["const"]] : [],
        )
      : [];
  const direct = named(schema["oneOf"]);
  if (direct.length > 0) return direct;
  if (Array.isArray(schema["enum"])) return schema["enum"].filter(isValue);
  // A setting that may be left out is one of the named values, or nothing.
  const either = schema["anyOf"];
  if (Array.isArray(either)) {
    for (const arm of either) {
      if (!isObject(arm)) continue;
      const within = enumerated(arm);
      if (within.length > 0) return within;
    }
  }
  return [];
}

/** What stands in for a setting that is left out, where the view says it outright. */
function decided(view: ConfigView, path: string): SettingValue | null {
  // Who draws the title bar is decided for the platform when the file does not say.
  return path === "window.title-bar" ? view.title_bar : null;
}

function sourceOf(view: ConfigView, path: string): SettingSource {
  const source = view.sources[path];
  const layer = isObject(source) ? source["layer"] : undefined;
  return layer === "user" || layer === "workspace" || layer === "profile" ? layer : "default";
}

function listed(list: SettingList, lists: SettingsLists): SettingChoice[] {
  switch (list) {
    case "themes":
      return lists.themes.map((theme) => ({ value: theme.id, label: theme.name }));
    case "fonts":
      return lists.fonts.map((font) => ({ value: font, label: font }));
    case "shells":
      return lists.shells.map((shell) => ({ value: shell.program, label: shell.name }));
    case "workspaces":
      return lists.workspaces.map((name) => ({ value: name, label: name }));
    case "profiles":
      return profileChoices(lists);
  }
}

function rowOf(found: Found, view: ConfigView, lists: SettingsLists): SettingRow {
  const { path, schema, annotation } = found;
  // A setting that can be left out and is empty is as good as left out: the file says
  // `default-profile = ""` for no profile at all.
  const leftOut = (held: SettingValue | null): SettingValue | null =>
    annotation.unset !== undefined && held === "" ? null : held;
  const value = leftOut(valueAt(view.config, path));
  const token = annotation.token === undefined ? undefined : valueOfToken(view, annotation.token);
  const fallback = leftOut(isValue(schema["default"]) ? schema["default"] : null);
  const effective = value ?? token ?? decided(view, path) ?? fallback;

  const choices: SettingChoice[] = [];
  const add = (choice: SettingChoice) => {
    if (!choices.some((held) => held.value === choice.value)) choices.push(choice);
  };
  if (annotation.control === "select" || annotation.control === "segmented") {
    for (const option of annotation.options ?? []) add(option);
    if (annotation.list) {
      // What the application starts with is offered even where the machine's own
      // list does not have it: the font it brings with it, for one.
      // Where the list has it, it goes by the name the list gives it.
      const fromList = listed(annotation.list, lists);
      if (fallback !== null && !fromList.some((choice) => choice.value === fallback)) {
        add({ value: fallback, label: String(fallback) });
      }
      for (const choice of fromList) add(choice);
    }
    if (choices.length === 0) {
      for (const allowed of enumerated(schema)) add({ value: allowed, label: worded(allowed) });
    }
    // A value written by hand is a choice like any other while it is the one in use.
    if (value !== null) add({ value, label: withUnit(value, annotation.unit) });
  }

  let unsetLabel: string | undefined;
  if (annotation.unset !== undefined) {
    // What stands in for a setting that is left out is known only while it is left
    // out: a token says what is in use, and a setting that is made is what is in use.
    const standsIn = value === null ? (token ?? null) : null;
    const named = choices.find((choice) => choice.value === standsIn)?.label;
    const shown = standsIn === null ? undefined : (named ?? withUnit(standsIn, annotation.unit));
    unsetLabel = shown === undefined ? annotation.unset : `${annotation.unset} (${shown})`;
    if (annotation.control !== "text") choices.unshift({ value: null, label: unsetLabel });
  }

  const source = sourceOf(view, path);
  const problem = view.problems.find((candidate) => candidate.path === path);
  const when = annotation.when;
  const shown =
    when === undefined || (valueAt(view.config, when.path) === null) === (when.is === "unset");

  return {
    path,
    label: annotation.label,
    ...(annotation.help === undefined ? {} : { help: annotation.help }),
    control: annotation.control,
    choices,
    value,
    effective,
    ...(unsetLabel === undefined ? {} : { unsetLabel }),
    source,
    overridden: source === "workspace" || source === "profile",
    ...(problem === undefined ? {} : { problem: describeProblem(problem) }),
    shown,
  };
}

function valueOfToken(view: ConfigView, token: string): SettingValue | undefined {
  const tokens: Readonly<Record<string, unknown>> = { ...view.tokens };
  const value = tokens[token];
  return isValue(value) ? value : undefined;
}

/**
 * The pages of settings a schema describes, filled in from the configuration in use.
 * Pages, headings and rows come in the order the schema gives them, never in the order
 * its keys happen to be in.
 */
export function settingsPages(
  schema: unknown,
  view: ConfigView,
  lists: SettingsLists = NO_LISTS,
): SettingsPage[] {
  if (!isObject(schema)) return [];
  const described = schema["x-lume-pages"];
  const pages = Array.isArray(described) ? described.flatMap((raw) => pageOf(raw) ?? []) : [];
  const settings = annotated(schema, "", []);

  return pages.map((page) => {
    const own = settings
      .filter((setting) => setting.annotation.page === page.id)
      .sort((a, b) => a.annotation.order - b.annotation.order || (a.path < b.path ? -1 : 1));
    // The headings the page names come first, in its order; a heading only a setting
    // names follows them, where its first setting falls.
    const headings = [...page.groups];
    for (const setting of own) {
      if (!headings.includes(setting.annotation.group)) headings.push(setting.annotation.group);
    }
    const groups = headings
      .map((label) => ({
        label,
        rows: own
          .filter((setting) => setting.annotation.group === label)
          .map((setting) => rowOf(setting, view, lists)),
      }))
      .filter((group) => group.rows.length > 0);
    return {
      id: page.id,
      label: page.label,
      icon: page.icon,
      group: page.group,
      blurb: page.blurb,
      groups,
    };
  });
}

/**
 * The rows that match what was typed into the search field, from every page: a row
 * matches by its name, what is said of it, its heading or its page.
 */
export function searchSettings(pages: readonly SettingsPage[], query: string): SettingsSearch {
  const wanted = query.trim().toLowerCase();
  if (wanted === "") return { groups: [], count: 0 };
  const groups: SettingsSearch["groups"][number][] = [];
  let count = 0;
  for (const page of pages) {
    for (const group of page.groups) {
      const rows = group.rows.filter(
        (row) =>
          row.shown &&
          [row.label, row.help ?? "", group.label, page.label]
            .join(" ")
            .toLowerCase()
            .includes(wanted),
      );
      if (rows.length === 0) continue;
      count += rows.length;
      groups.push({
        label: [page.group, page.label, group.label].join(PLACE_SEPARATOR),
        page: page.id,
        rows,
      });
    }
  }
  return { groups, count };
}

/** The choice of a row that is the one in use, as its place among the row's choices. */
export function chosenIndex(row: SettingRow): number {
  const exact = row.choices.findIndex((choice) => choice.value === row.value);
  if (exact >= 0) return exact;
  // A setting that is left out and has no choice for that shows what it comes to.
  return row.choices.findIndex((choice) => choice.value === row.effective);
}

/** The change to the user's file that choosing one of a row's choices asks for. */
export function writeChoice(row: SettingRow, choice: SettingChoice): SettingWrite {
  return choice.value === null
    ? { op: "unset", path: row.path }
    : { op: "set", path: row.path, value: choice.value };
}

/** The change that turning a row's switch on or off asks for. */
export function writeSwitch(row: SettingRow, on: boolean): SettingWrite {
  return { op: "set", path: row.path, value: on };
}

/** The change that typing into a row's field asks for: nothing typed leaves it out. */
export function writeText(row: SettingRow, typed: string): SettingWrite {
  const value = typed.trim();
  return value === "" ? { op: "unset", path: row.path } : { op: "set", path: row.path, value };
}
