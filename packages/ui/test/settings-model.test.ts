import { readFileSync } from "node:fs";

import { DEFAULT_TOKENS } from "@lume/tokens";
import { describe, expect, it } from "vitest";

import {
  chosenIndex,
  searchSettings,
  settingsPages,
  writeChoice,
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
    ]);
    expect(appearance.groups[1]!.rows.map((held) => held.path)).toEqual([
      "appearance.card-radius",
      "appearance.background-image",
      "window.title-bar",
    ]);
    // A heading only a setting names follows the ones its page names.
    expect(pages[1]!.groups.map((group) => group.label)).toEqual(["Cursor", "Shell"]);
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

  it("offers a setting left out as a choice that names what stands in for it", () => {
    const radius = row(settingsPages(SCHEMA, view(), LISTS), "appearance.card-radius");
    const standsIn = DEFAULT_TOKENS["radius.card"];
    expect(radius.value).toBeNull();
    expect(radius.effective).toBe(standsIn);
    expect(radius.unsetLabel).toBe(`Theme\u2019s (${standsIn} px)`);
    expect(radius.choices[0]).toEqual({ value: null, label: radius.unsetLabel });
    expect(chosenIndex(radius)).toBe(0);
    expect(writeChoice(radius, radius.choices[0]!)).toEqual({
      op: "unset",
      path: "appearance.card-radius",
    });
    expect(writeChoice(radius, radius.choices[2]!)).toEqual({
      op: "set",
      path: "appearance.card-radius",
      value: 6,
    });
  });

  it("adds a value written by hand to the choices while it is the one in use", () => {
    const radius = row(
      settingsPages(SCHEMA, view({}, { "card-radius": 13 }), LISTS),
      "appearance.card-radius",
    );
    expect(radius.choices.at(-1)).toEqual({ value: 13, label: "13 px" });
    expect(chosenIndex(radius)).toBe(radius.choices.length - 1);
    expect(radius.effective).toBe(13);
    // What would stand in for it is not known while it is made: the token says 13 too.
    expect(radius.choices[0]).toEqual({ value: null, label: "Theme’s" });
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
    expect(byName.count).toBe(1);
    expect(byName.groups).toHaveLength(1);
    expect(byName.groups[0]).toMatchObject({
      label: "Application \u203a Appearance \u203a Panes",
      page: "appearance",
    });
    expect(searchSettings(pages, "IMAGE FILE").groups[0]!.rows[0]!.path).toBe(
      "appearance.background-image",
    );
    expect(searchSettings(pages, "cursor").count).toBe(2);
    expect(searchSettings(pages, "terminal").count).toBe(5);
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
    expect(pages.map((page) => page.id)).toEqual(["general", "appearance", "terminal"]);
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
        }
      }
    }
  });
});
