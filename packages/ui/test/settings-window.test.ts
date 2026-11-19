import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { createStore } from "zustand/vanilla";

import type { ConfigState } from "../src/config/store.ts";
import type { ConfigView } from "../src/core/types.ts";
import type { AddonEntry } from "../src/settings/addons.ts";
import type { SettingsApp, ShownPage } from "../src/settings/boot.ts";
import type { SettingsLists, SettingsPage } from "../src/settings/model.ts";
import { SettingsRegistry } from "../src/settings/registry.ts";
import { SettingsWindow } from "../src/settings/SettingsWindow.tsx";
import { NoticeStore } from "../src/shell/notices.ts";
import { createWindowStateStore } from "../src/window/state.ts";
import { LISTS, SCHEMA, view } from "./settings-support.ts";

// The notices follow a store that only a page can follow; they are not what is drawn here.
vi.mock("../src/shell/Notices.tsx", () => ({ Notices: () => null }));

/** A settings window's page, with nothing behind it. */
function app(
  held: ConfigView,
  page: string | null = null,
  addons: readonly AddonEntry[] | null = [],
): SettingsApp {
  return {
    // What is drawn without a page is what a store starts with, so this one starts
    // with the configuration.
    config: createStore<ConfigState>()(() => ({ view: held, set: () => false })),
    client: {} as SettingsApp["client"],
    schema: SCHEMA,
    lists: createStore<SettingsLists>()(() => LISTS),
    registry: new SettingsRegistry(),
    notices: new NoticeStore(),
    windowState: createWindowStateStore(),
    shown: createStore<ShownPage>()(() => ({ page, asked: 0, show: () => undefined })),
    addons: createStore<{ readonly list: readonly AddonEntry[] | null }>()(() => ({
      list: addons,
    })),
    refreshAddons: () => undefined,
    pickFolder: () => Promise.resolve(null),
    write: () => Promise.resolve(),
    openFile: () => undefined,
    close: () => undefined,
  };
}

function markup(made: SettingsApp): string {
  return renderToStaticMarkup(createElement(SettingsWindow, { app: made }));
}

describe("the settings window", () => {
  it("lists the pages under their heading and shows the first", () => {
    const html = markup(app(view()));
    expect(html).toContain('<nav class="settings-nav" aria-label="Settings sections">');
    expect(html).toContain("<h4>Application</h4>");
    expect(html).toMatch(/aria-current="page" tabindex="0">.*?Appearance<\/span>/);
    expect(html).toMatch(/class="settings-nav-item" tabindex="-1">.*?Terminal<\/span>/);
    expect(html).toContain("<h1>Appearance</h1>");
    expect(html).toContain('<p class="settings-intro">Theme, colours and type.</p>');
    expect(html).toContain("<h2>Theme</h2>");
    expect(html).toContain("<h2>Panes</h2>");
    expect(html).toContain('data-path="appearance.card-radius"');
    expect(html).not.toContain('data-path="terminal.cursor-style"');
    // Nothing is styled by an attribute of its own.
    expect(html).not.toContain("style=");
  });

  it("shows the page it was opened at", () => {
    const html = markup(app(view(), "terminal"));
    expect(html).toContain("<h1>Terminal</h1>");
    expect(html).toContain('data-path="terminal.cursor-style"');
    expect(html).not.toContain('data-path="appearance.card-radius"');
  });

  it("leaves out a row whose time has not come", () => {
    expect(markup(app(view()))).toContain('data-path="appearance.theme-dark"');
    expect(markup(app(view({}, { theme: "ember" })))).not.toContain(
      'data-path="appearance.theme-dark"',
    );
  });

  it("draws a title bar of its own only where the page draws the title bar", () => {
    const drawn = markup(app(view({ title_bar: "custom" })));
    expect(drawn).toContain('<header class="settings-titlebar">');
    expect(drawn).toContain('aria-label="Close window"');
    const system = markup(app(view({ title_bar: "system" })));
    expect(system).not.toContain("settings-titlebar");
    // A Mac keeps its own three buttons over the bar.
    const mac = markup(app(view({ title_bar: "custom", platform: "macos" })));
    expect(mac).toContain("settings-titlebar");
    expect(mac).not.toContain('aria-label="Close window"');
  });

  it("offers the search field and the way to the file", () => {
    const html = markup(app(view()));
    expect(html).toContain('aria-label="Search settings"');
    expect(html).toContain(">Open config.toml</button>");
  });

  it("lists a page handed over after the application's own, with what it shows beside its rows", () => {
    const made = app(view(), "metrics");
    const extra: SettingsPage = {
      id: "metrics",
      label: "Performance metrics",
      icon: "metrics",
      group: "Add-ons",
      meta: "v1.2",
      blurb: "Meters in the status bar.",
      groups: [],
      preview: () => createElement("p", null, "three meters"),
    };
    made.registry.register(extra);
    const html = markup(made);
    expect(html).toContain("<h4>Add-ons</h4>");
    expect(html).toContain('<span class="settings-nav-meta">v1.2</span>');
    expect(html).toContain("<h1>Performance metrics</h1>");
    expect(html).toContain('<span class="settings-meta">v1.2</span>');
    expect(html).toContain(
      '<aside class="settings-preview" aria-label="Live preview"><p>three meters</p></aside>',
    );
    // A page that shows nothing beside its rows leaves the column empty.
    expect(markup(app(view()))).not.toContain("settings-preview");
  });

  it("manages the plugins on a page of its own, and gives each its page in the order they load in", () => {
    const tools: AddonEntry = {
      id: "acme.tools",
      manifestId: "acme.tools",
      name: "Tools",
      version: "0.4.0",
      description: "Jobs and things.",
      enabled: true,
      state: { ready: true, reason: null },
      modified: false,
      missing: false,
      source: "D:/plugins/tools",
      dependencies: [{ id: "acme.core", optional: false, bound: "acme.core-2", rebound: true }],
      permissions: [
        {
          what: "Run these programs, with any arguments: git",
          warning: "A program such as git can do anything you can.",
        },
      ],
      later: ["Read what you copy"],
      health: { running: true, starts: 2, breaches: 0, lastProblem: null, warnings: [] },
      log: ["info started"],
      audit: [
        {
          time: "2026-11-19T10:00:00Z",
          capability: "process.run",
          scope: "git",
          outcome: "allowed",
        },
        {
          time: "2026-11-19T10:01:00Z",
          capability: "net",
          scope: "example.org",
          outcome: "denied",
        },
      ],
    };
    const changed: AddonEntry = {
      ...tools,
      id: "acme.changed",
      manifestId: "acme.changed",
      name: "Changed",
      modified: true,
      dependencies: [],
      audit: [],
      log: [],
    };
    const manage = markup(app(view(), "addons", [tools, changed]));
    expect(manage).toContain("<h4>Add-ons</h4>");
    expect(manage).toContain("<h1>Manage add-ons</h1>");
    expect(manage).toContain("Install from folder…");
    expect(manage).toContain('data-addon="acme.tools"');
    expect(manage).toContain("Uses acme.core as installed under acme.core-2.");
    expect(manage).toContain("so it is not used. Install it again to use it.");
    expect(manage).toMatch(/aria-label="Load Tools earlier" disabled=""/);
    expect(manage.indexOf("Tools</span>")).toBeLessThan(manage.indexOf("Changed</span>"));

    const page = markup(app(view(), "addon:acme.tools", [tools]));
    expect(page).toContain("<h1>Tools</h1>");
    expect(page).toContain("<h2>Permissions</h2>");
    expect(page).toContain("Run these programs, with any arguments: git");
    expect(page).toContain("It may ask you later to:");
    expect(page).toContain("<h2>Health</h2>");
    expect(page).toContain("Running");
    expect(page).toContain("<h2>Activity</h2>");
    expect(page).toContain("Refused net (example.org)");
    expect(page).toContain("info started");
  });
});
