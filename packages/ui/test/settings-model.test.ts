import { readFileSync } from "node:fs";

import { DEFAULT_TOKENS } from "@lume/tokens";
import { describe, expect, it } from "vitest";

import {
  chosenIndex,
  searchSettings,
  settingsPages,
  shownNumber,
  stepNumber,
  typedColor,
  writeChoice,
  writeNumber,
  writeSwatch,
  writeSwitch,
  writeText,
  type SettingRow,
  type SettingsPage,
} from "../src/settings/model.ts";
import { allPages, navOf, SettingsRegistry } from "../src/settings/registry.ts";
import { LISTS, SCHEMA, view } from "./settings-support.ts";

function row(pages: readonly SettingsPage[], path: string): SettingRow {
  for (const page of pages) {
    for (const group of page.groups) {
      const found = group.rows.find((candidate) => candidate.path === path);
      if (found) return found;
    }
  }
  throw new Error(`no row for ${path}`);
}

describe("the settings as pages of rows", () => {
  it("lists pages, headings and rows in the order the schema gives, not that of its keys", () => {
    const pages = settingsPages(SCHEMA, view(), LISTS);
    expect(pages.map((page) => page.id)).toEqual(["appearance", "terminal"]);
    const appearance = pages[0]!;
    expect(appearance.groups.map((group) => group.label)).toEqual(["Theme", "Panes"]);
    expect(appearance.groups[0]!.rows.map((held) => held.path)).toEqual([
      "appearance.theme",
      "appearance.theme-dark",
      "font.family",
      "appearance.accent",
    ]);
    expect(appearance.groups[1]!.rows.map((held) => held.path)).toEqual([
      "appearance.card-radius",
      "appearance.dim-unfocused",
      "appearance.background-image",
      "window.title-bar",
      "window.corners",
    ]);
    // A heading only a setting names follows the ones its page names.
    expect(pages[1]!.groups.map((group) => group.label)).toEqual(["Cursor", "Buffer", "Shell"]);
    expect(pages[1]!.groups[0]!.rows.map((held) => held.label)).toEqual([
      "Cursor style",
      "Blinking cursor",
    ]);
  });

  it("carries a page's own words, and shows nothing that is not marked to be shown", () => {
    const pages = settingsPages(SCHEMA, view(), LISTS);
    expect(pages[0]).toMatchObject({
      label: "Appearance",
      icon: "appearance",
      group: "Application",
      blurb: "Theme, colours and type.",
    });
    const paths = pages.flatMap((page) => page.groups.flatMap((group) => group.rows));
    expect(paths.some((held) => held.path === "appearance.tokens")).toBe(false);
  });

  it("types a number left out as nothing, and names what stands in for it", () => {
    const radius = row(settingsPages(SCHEMA, view(), LISTS), "appearance.card-radius");
    const standsIn = DEFAULT_TOKENS["radius.card"];
    expect(radius.value).toBeNull();
    expect(radius.effective).toBe(standsIn);
    expect(radius.unsetLabel).toBe(`Theme’s (${standsIn} px)`);
    expect(radius.choices).toEqual([]);
    expect(radius.number).toEqual({
      min: 0,
      max: 64,
      step: 1,
      unit: "px",
      scale: 1,
      integer: false,
    });
    expect(shownNumber(radius, radius.value)).toBe("");
    expect(shownNumber(radius, radius.effective)).toBe(String(standsIn));
    expect(writeNumber(radius, "  ")).toEqual({ op: "unset", path: "appearance.card-radius" });
    expect(writeNumber(radius, "6")).toEqual({
      op: "set",
      path: "appearance.card-radius",
      value: 6,
    });
  });

  it("offers a colour's swatches as they look on the theme in use, and the theme's own", () => {
    const dark = row(settingsPages(SCHEMA, view(), LISTS), "appearance.accent");
    expect(dark.control).toBe("swatch");
    expect(dark.value).toBeNull();
    expect(dark.swatches).toEqual([
      { value: "mint", label: "Mint", color: "#7fe3b8" },
      { value: "blue", label: "Blue", color: "#7aa7ff" },
    ]);
    // Leaving it out is a choice like the swatches.
    expect(dark.choices).toEqual([{ value: null, label: "Theme’s" }]);
    const light = row(
      settingsPages(SCHEMA, view({ theme: { id: "lume-light", scheme: "light" } }), LISTS),
      "appearance.accent",
    );
    expect(light.swatches?.map((swatch) => swatch.color)).toEqual(["#147a55", "#2f6fd0"]);
    const own = row(
      settingsPages(SCHEMA, view({}, { accent: "#336699" }), LISTS),
      "appearance.accent",
    );
    expect(own.value).toBe("#336699");
  });

  it("writes a colour chosen, or one typed as a person writes it", () => {
    const accent = row(settingsPages(SCHEMA, view(), LISTS), "appearance.accent");
    expect(writeSwatch(accent, "blue")).toEqual({
      op: "set",
      path: "appearance.accent",
      value: "blue",
    });
    expect(writeSwatch(accent, null)).toEqual({ op: "unset", path: "appearance.accent" });
    expect(typedColor(" #A0b1C2 ")).toBe("#a0b1c2");
    expect(typedColor("a0b1c2")).toBe("#a0b1c2");
    // Three digits stand for six: each one twice.
    expect(typedColor("#7AF")).toBe("#77aaff");
    for (const wrong of ["", "#ab", "#abcd", "#a0b1c2ff", "teal", "#ggg000"]) {
      expect(typedColor(wrong), wrong).toBeNull();
    }
  });

  it("holds a typed number to its bounds and reads it as a person writes it", () => {
    const radius = row(
      settingsPages(SCHEMA, view({}, { "card-radius": 13 }), LISTS),
      "appearance.card-radius",
    );
    expect(radius.value).toBe(13);
    expect(shownNumber(radius, radius.value)).toBe("13");
    // What would stand in for it is not known while it is made: the token says 13 too.
    expect(radius.unsetLabel).toBe("Theme’s");
    const set = (typed: string) => writeNumber(radius, typed);
    expect(set("80")).toMatchObject({ value: 64 });
    expect(set("-3")).toMatchObject({ value: 0 });
    expect(set("12,5 px")).toMatchObject({ value: 12.5 });
    expect(set("7px")).toMatchObject({ value: 7 });
    expect(set(".5")).toMatchObject({ value: 0.5 });
    expect(set("0.12345")).toMatchObject({ value: 0.123 });
    expect(set("twelve")).toBeNull();
    expect(set("0x10")).toBeNull();
    expect(set("1e3")).toBeNull();
    expect(stepNumber(radius, null, 1)).toMatchObject({ value: 14 });
    expect(stepNumber(radius, 63, 10)).toMatchObject({ value: 64 });
    expect(stepNumber(radius, 2, -10)).toMatchObject({ value: 0 });
  });

  it("shows a share in hundredths and writes it from 0 to 1", () => {
    const dim = row(
      settingsPages(SCHEMA, view({}, { "dim-unfocused": 0.15 }), LISTS),
      "appearance.dim-unfocused",
    );
    expect(dim.number).toMatchObject({ min: 0, max: 100, unit: "%", scale: 100 });
    expect(shownNumber(dim, dim.value)).toBe("15");
    expect(writeNumber(dim, "35")).toMatchObject({ value: 0.35 });
    expect(writeNumber(dim, "35 %")).toMatchObject({ value: 0.35 });
    expect(writeNumber(dim, "150")).toMatchObject({ value: 1 });
    expect(writeNumber(dim, "12.5")).toMatchObject({ value: 0.125 });
    expect(stepNumber(dim, null, 1)).toMatchObject({ value: 0.16 });
    expect(stepNumber(dim, 35, -10)).toMatchObject({ value: 0.25 });
  });

  it("writes a whole number for a setting that takes only whole numbers", () => {
    const lines = row(settingsPages(SCHEMA, view(), LISTS), "terminal.scrollback-lines");
    expect(lines.number).toMatchObject({ integer: true, step: 1000, unit: "lines" });
    expect(writeNumber(lines, "1234.6")).toMatchObject({ value: 1235 });
    expect(stepNumber(lines, null, 1)).toMatchObject({ value: 11_000 });
    expect(stepNumber(lines, 99_500, 1)).toMatchObject({ value: 100_000 });
    // A setting that cannot be left out keeps what it has when nothing is typed.
    expect(writeNumber(lines, "")).toBeNull();
  });

  it("shows a row meant for one platform only on that platform", () => {
    const here = settingsPages(SCHEMA, view(), LISTS);
    expect(row(here, "window.corners").shown).toBe(true);
    const elsewhere = settingsPages(SCHEMA, view({ platform: "macos" }), LISTS);
    expect(row(elsewhere, "window.corners").shown).toBe(false);
    expect(searchSettings(elsewhere, "window corners").count).toBe(0);
    expect(searchSettings(here, "window corners").count).toBe(1);
  });

  it("takes choices from the machine's lists, the value the application starts with first", () => {
    const pages = settingsPages(SCHEMA, view(), LISTS);
    expect(row(pages, "font.family").choices.map((choice) => choice.value)).toEqual([
      "JetBrains Mono",
      "Cascadia Mono",
      "Consolas",
    ]);
    const theme = row(pages, "appearance.theme");
    expect(theme.choices).toEqual([
      { value: null, label: "Follow system" },
      { value: "lume-dark", label: "Lume Dark" },
      { value: "lume-light", label: "Lume Light" },
      { value: "ember", label: "Ember" },
    ]);
    expect(row(pages, "terminal.shell").choices).toEqual([
      { value: null, label: "System default" },
      { value: "pwsh.exe", label: "PowerShell" },
    ]);
    // A value the application starts with that the list holds goes by the list's name
    // for it, and keeps its place in the list.
    expect(row(pages, "appearance.theme-dark").choices).toEqual([
      { value: "lume-dark", label: "Lume Dark" },
      { value: "lume-light", label: "Lume Light" },
      { value: "ember", label: "Ember" },
    ]);
  });

  it("lists the profiles there are to choose from, by kind, without the hidden ones", () => {
    const pages = settingsPages(SCHEMA, view(), LISTS);
    expect(row(pages, "default-profile").choices).toEqual([
      { value: null, label: "System default" },
      { value: "box", label: "SSH: Box" },
      { value: "auto.wsl.Ubuntu", label: "WSL: Ubuntu" },
    ]);
  });

  it("shows a setting that can be left out and is empty as left out", () => {
    // The configuration in use carries every setting, an empty one as nothing at all.
    const empty = view({ config: { ...view().config, "default-profile": "" } });
    const profile = row(settingsPages(SCHEMA, empty, LISTS), "default-profile");
    expect(profile.value).toBeNull();
    expect(profile.choices.some((choice) => choice.value === "")).toBe(false);
    expect(profile.choices[chosenIndex(profile)]).toEqual({
      value: null,
      label: "System default",
    });
    expect(writeChoice(profile, profile.choices[0]!)).toEqual({
      op: "unset",
      path: "default-profile",
    });
    const base = view().config;
    const noShell = view({ config: { ...base, terminal: { ...base.terminal, shell: "" } } });
    const shell = row(settingsPages(SCHEMA, noShell, LISTS), "terminal.shell");
    expect(shell.choices).toEqual([
      { value: null, label: "System default" },
      { value: "pwsh.exe", label: "PowerShell" },
    ]);
    expect(chosenIndex(shell)).toBe(0);

    const chosen = view({ config: { ...view().config, "default-profile": "box" } });
    const set = row(settingsPages(SCHEMA, chosen, LISTS), "default-profile");
    expect(set.value).toBe("box");
    expect(set.choices[chosenIndex(set)]).toEqual({ value: "box", label: "SSH: Box" });
  });

  it("reads the choices of a setting the schema names one by one", () => {
    const style = row(settingsPages(SCHEMA, view(), LISTS), "terminal.cursor-style");
    expect(style.choices).toEqual([
      { value: "block", label: "Block" },
      { value: "bar", label: "Bar" },
      { value: "underline", label: "Underline" },
    ]);
    expect(chosenIndex(style)).toBe(1);
  });

  it("shows what was decided for a setting that is left out and has no choice for that", () => {
    const pages = settingsPages(SCHEMA, view({ title_bar: "system" }), LISTS);
    const bar = row(pages, "window.title-bar");
    expect(bar.value).toBeNull();
    expect(bar.effective).toBe("system");
    expect(bar.choices.map((choice) => choice.label)).toEqual(["Lume", "System"]);
    expect(chosenIndex(bar)).toBe(1);
  });

  it("shows a row that follows a switch only while the switch is on", () => {
    const schema = structuredClone(SCHEMA) as typeof SCHEMA;
    const terminal = schema.properties.terminal.properties as Record<string, unknown>;
    terminal["history-paging"] = {
      type: "boolean",
      default: false,
      "x-lume-settings": {
        page: "terminal",
        group: "Buffer",
        order: 20,
        label: "Fetch older lines when needed",
        control: "switch",
      },
    };
    terminal["window-history-lines"] = {
      type: "integer",
      minimum: 100,
      maximum: 100_000,
      default: 1000,
      "x-lume-settings": {
        page: "terminal",
        group: "Buffer",
        order: 30,
        label: "Lines kept in the window",
        control: "number",
        unit: "lines",
        step: 1000,
        when: { path: "terminal.history-paging", is: "on" },
      },
    };
    const withPaging = (paging: boolean | undefined) => {
      const base = view();
      const config = {
        ...base.config,
        terminal: {
          ...base.config.terminal,
          ...(paging === undefined ? {} : { "history-paging": paging }),
        },
      };
      return settingsPages(schema, { ...base, config }, LISTS);
    };
    expect(row(withPaging(true), "terminal.window-history-lines").shown).toBe(true);
    expect(row(withPaging(false), "terminal.window-history-lines").shown).toBe(false);
    // A core that does not say has it off.
    expect(row(withPaging(undefined), "terminal.window-history-lines").shown).toBe(false);
    expect(row(withPaging(false), "terminal.history-paging").shown).toBe(true);
  });

  it("shows a row only while the setting it depends on is as the row says", () => {
    const following = settingsPages(SCHEMA, view(), LISTS);
    expect(row(following, "appearance.theme-dark").shown).toBe(true);
    const fixed = settingsPages(SCHEMA, view({}, { theme: "ember" }), LISTS);
    expect(row(fixed, "appearance.theme-dark").shown).toBe(false);
    expect(row(fixed, "appearance.theme").shown).toBe(true);
  });

  it("says where a value came from, and when the user's file does not decide it", () => {
    const sources = {
      "terminal.cursor-style": { layer: "user", file: "config.toml", line: 4 },
      "terminal.cursor-blink": { layer: "workspace", name: "notes", file: "notes.toml", line: 2 },
    };
    const pages = settingsPages(SCHEMA, view({ sources }), LISTS);
    expect(row(pages, "terminal.cursor-style")).toMatchObject({
      source: "user",
      overridden: false,
    });
    expect(row(pages, "terminal.cursor-blink")).toMatchObject({
      source: "workspace",
      overridden: true,
    });
    expect(row(pages, "font.family")).toMatchObject({ source: "default", overridden: false });
  });

  it("puts what is wrong with a setting in the user's files on its row", () => {
    const problems = [
      {
        kind: "invalid",
        path: "terminal.cursor-style",
        file: "C:\\Users\\someone\\config.toml",
        line: 7,
        column: 1,
        message: "not one of block, bar, underline",
        suggestion: null,
      },
    ];
    const pages = settingsPages(SCHEMA, view({ problems }), LISTS);
    expect(row(pages, "terminal.cursor-style").problem).toBe(
      "config.toml:7: not one of block, bar, underline",
    );
    expect(row(pages, "terminal.cursor-blink").problem).toBeUndefined();
  });

  it("turns a switch and a typed field into changes to the file", () => {
    const pages = settingsPages(SCHEMA, view(), LISTS);
    const blink = row(pages, "terminal.cursor-blink");
    expect(blink.effective).toBe(false);
    expect(writeSwitch(blink, true)).toEqual({
      op: "set",
      path: "terminal.cursor-blink",
      value: true,
    });
    const picture = row(pages, "appearance.background-image");
    expect(picture.unsetLabel).toBe("None");
    expect(picture.choices).toEqual([]);
    expect(writeText(picture, "  C:\\pictures\\hills.png ")).toEqual({
      op: "set",
      path: "appearance.background-image",
      value: "C:\\pictures\\hills.png",
    });
    expect(writeText(picture, "   ")).toEqual({ op: "unset", path: "appearance.background-image" });
  });

  it("makes no pages of something that is not a schema", () => {
    expect(settingsPages(null, view(), LISTS)).toEqual([]);
    expect(settingsPages({ properties: {} }, view(), LISTS)).toEqual([]);
  });
});

describe("searching the settings", () => {
  const pages = settingsPages(SCHEMA, view(), LISTS);

  it("finds rows by name, by what is said of them, by heading and by page", () => {
    const byName = searchSettings(pages, "corners");
    expect(byName.count).toBe(2);
    expect(byName.groups).toHaveLength(1);
    expect(byName.groups[0]).toMatchObject({
      label: "Application \u203a Appearance \u203a Panes",
      page: "appearance",
    });
    expect(searchSettings(pages, "IMAGE FILE").groups[0]!.rows[0]!.path).toBe(
      "appearance.background-image",
    );
    expect(searchSettings(pages, "cursor").count).toBe(2);
    expect(searchSettings(pages, "terminal").count).toBe(6);
  });

  it("finds nothing for nothing typed, and counts nothing it does not show", () => {
    expect(searchSettings(pages, "   ")).toEqual({ groups: [], count: 0 });
    expect(searchSettings(pages, "no such setting")).toEqual({ groups: [], count: 0 });
    const fixed = settingsPages(SCHEMA, view({}, { theme: "ember" }), LISTS);
    expect(searchSettings(fixed, "dark mode").count).toBe(0);
    expect(searchSettings(pages, "dark mode").count).toBe(1);
  });
});

describe("the pages the settings window draws", () => {
  const core = settingsPages(SCHEMA, view(), LISTS);
  const extra: SettingsPage = {
    id: "metrics",
    label: "Performance metrics",
    icon: "metrics",
    group: "Add-ons",
    meta: "v1.2",
    blurb: "Meters in the status bar.",
    groups: [],
  };

  it("lists each heading that has pages over its pages", () => {
    const nav = navOf(allPages(core, [extra]));
    expect(
      nav.map((entry) => (entry.kind === "heading" ? `# ${entry.label}` : entry.page.id)),
    ).toEqual(["# Application", "appearance", "terminal", "# Add-ons", "metrics"]);
  });

  it("puts a heading nobody foresaw after the ones that were", () => {
    const odd: SettingsPage = { ...extra, id: "odd", group: "Experiments" };
    const nav = navOf([odd, ...core]);
    expect(nav.flatMap((entry) => (entry.kind === "heading" ? [entry.label] : []))).toEqual([
      "Application",
      "Experiments",
    ]);
  });

  it("passes over a page handed over under the id of one of the application's own", () => {
    const pages = allPages(core, [{ ...extra, id: "terminal" }, extra]);
    expect(pages.map((page) => page.id)).toEqual(["appearance", "terminal", "metrics"]);
    expect(pages[1]).toBe(core[1]);
  });

  it("holds the pages handed over, and tells whoever listens when they change", () => {
    const registry = new SettingsRegistry();
    let told = 0;
    registry.subscribe(() => (told += 1));
    const remove = registry.register(extra);
    expect(registry.current()).toEqual([extra]);
    const newer = { ...extra, meta: "v1.3" };
    registry.register(newer);
    expect(registry.current()).toEqual([newer]);
    // The page that was replaced is no longer there to take away.
    remove();
    expect(registry.current()).toEqual([newer]);
    expect(told).toBe(2);
  });
});

describe("the configuration's own schema", () => {
  const schema: unknown = JSON.parse(
    readFileSync(new URL("../../../schemas/config.schema.json", import.meta.url), "utf8"),
  );
  const pages = settingsPages(schema, view(), LISTS);

  // The schema says nothing of pages until it has been generated with them.
  it.skipIf(pages.length === 0)("describes pages whose every row can be drawn", () => {
    expect(pages.map((page) => page.id)).toEqual([
      "general",
      "appearance",
      "terminal",
      "notifications",
    ]);
    for (const page of pages) {
      expect(page.groups.length, page.id).toBeGreaterThan(0);
      for (const group of page.groups) {
        for (const held of group.rows) {
          if (held.control === "select" || held.control === "segmented") {
            expect(held.choices.length, held.path).toBeGreaterThan(0);
          }
          if (held.control === "switch") {
            expect(typeof held.effective, held.path).toBe("boolean");
          }
          if (held.control === "swatch") {
            expect(held.swatches?.length, held.path).toBeGreaterThan(0);
          }
          if (held.control === "number") {
            expect(held.number, held.path).toBeDefined();
            expect(Number.isFinite(held.number!.min), held.path).toBe(true);
            expect(Number.isFinite(held.number!.max), held.path).toBe(true);
            expect(typeof held.effective, held.path).toBe("number");
          }
        }
      }
    }
  });

  it.skipIf(pages.length === 0)(
    "lists the history rows in order, the time a hidden pane waits only while it lets go",
    () => {
      const terminal = pages.find((page) => page.id === "terminal");
      const history = terminal?.groups.find((group) => group.label === "History");
      expect(history?.rows.map((held) => held.path)).toEqual([
        "terminal.scrollback-lines",
        "terminal.history-paging",
        "terminal.window-history-lines",
        "terminal.history-release-seconds",
        "terminal.hidden-release",
        "terminal.hidden-release-seconds",
      ]);
      const shown = (releasing: boolean | undefined) => {
        const base = view();
        const config = {
          ...base.config,
          terminal: {
            ...base.config.terminal,
            ...(releasing === undefined ? {} : { "hidden-release": releasing }),
          },
        };
        return row(
          settingsPages(schema, { ...base, config }, LISTS),
          "terminal.hidden-release-seconds",
        ).shown;
      };
      expect(shown(true)).toBe(true);
      expect(shown(false)).toBe(false);
      // A core that does not say has it off.
      expect(shown(undefined)).toBe(false);
    },
  );
});
