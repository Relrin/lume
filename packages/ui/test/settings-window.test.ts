import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { createStore } from "zustand/vanilla";

import type { ConfigState } from "../src/config/store.ts";
import type { ConfigView } from "../src/core/types.ts";
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
function app(held: ConfigView, page: string | null = null): SettingsApp {
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
});
