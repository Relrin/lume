/**
 * Shared by the settings tests: a schema in the shape the configuration's has, small
 * enough to read, the lists a machine would add to it, and a configuration to show.
 */

import { DEFAULT_TOKENS } from "@lume/tokens";

import type { ConfigView } from "../src/core/types.ts";
import type { SettingsLists } from "../src/settings/model.ts";
import { DEVBOX, NAS } from "./rex-fixtures.ts";
import { settings } from "./config-support.ts";

/** A schema in the shape the configuration's has, small enough to read. */
export const SCHEMA = {
  type: "object",
  "x-lume-pages": [
    {
      id: "appearance",
      label: "Appearance",
      icon: "appearance",
      group: "Application",
      blurb: "Theme, colours and type.",
      groups: ["Theme", "Panes"],
    },
    {
      id: "terminal",
      label: "Terminal",
      icon: "terminal",
      group: "Application",
      blurb: "Cursor and scrollback.",
      groups: ["Cursor", "Buffer"],
    },
  ],
  // The keys are out of order on purpose: nothing may depend on their order.
  properties: {
    // A setting at the top of the file, outside every table, which is empty when unset.
    "default-profile": {
      type: "string",
      default: "",
      "x-lume-settings": {
        page: "terminal",
        group: "Shell",
        order: 40,
        label: "Default terminal",
        control: "select",
        list: "profiles",
        unset: "System default",
      },
    },
    rex: {
      type: "object",
      properties: {
        "start-on": {
          type: "string",
          default: "",
          "x-lume-settings": {
            page: "terminal",
            group: "Shell",
            order: 50,
            label: "Start on",
            control: "select",
            list: "rex-hosts",
            unset: "Your workspace",
          },
        },
      },
    },
    terminal: {
      type: "object",
      properties: {
        "cursor-blink": {
          type: "boolean",
          default: true,
          "x-lume-settings": {
            page: "terminal",
            group: "Cursor",
            order: 20,
            label: "Blinking cursor",
            control: "switch",
          },
        },
        "cursor-style": {
          default: "block",
          oneOf: [
            { const: "block", type: "string" },
            { const: "bar", type: "string" },
            { const: "underline", type: "string" },
          ],
          "x-lume-settings": {
            page: "terminal",
            group: "Cursor",
            order: 10,
            label: "Cursor style",
            control: "segmented",
          },
        },
        "scrollback-lines": {
          type: "integer",
          minimum: 0,
          maximum: 100_000,
          default: 10_000,
          "x-lume-settings": {
            page: "terminal",
            group: "Buffer",
            order: 10,
            label: "Scrollback",
            control: "number",
            unit: "lines",
            step: 1000,
          },
        },
        shell: {
          type: ["string", "null"],
          "x-lume-settings": {
            page: "terminal",
            group: "Shell",
            order: 30,
            label: "Default shell",
            control: "select",
            list: "shells",
            unset: "System default",
          },
        },
      },
    },
    window: {
      type: "object",
      properties: {
        "title-bar": {
          anyOf: [
            {
              oneOf: [
                { const: "custom", type: "string" },
                { const: "system", type: "string" },
              ],
            },
            { type: "null" },
          ],
          "x-lume-settings": {
            page: "appearance",
            group: "Panes",
            order: 90,
            label: "Title bar",
            control: "segmented",
            options: [
              { value: "custom", label: "Lume" },
              { value: "system", label: "System" },
            ],
          },
        },
        corners: {
          default: "round",
          oneOf: [
            { const: "round", type: "string" },
            { const: "square", type: "string" },
          ],
          "x-lume-settings": {
            page: "appearance",
            group: "Panes",
            order: 95,
            label: "Window corners",
            control: "segmented",
            options: [
              { value: "round", label: "Round" },
              { value: "square", label: "Square" },
            ],
            platform: "windows",
          },
        },
      },
    },
    font: {
      type: "object",
      properties: {
        family: {
          type: "string",
          default: "JetBrains Mono",
          "x-lume-settings": {
            page: "appearance",
            group: "Theme",
            order: 30,
            label: "Terminal font",
            control: "select",
            list: "fonts",
          },
        },
        "ui-family": {
          type: "string",
          default: "Inter",
          "x-lume-settings": {
            page: "appearance",
            group: "Theme",
            order: 25,
            label: "Interface font",
            control: "select",
            options: [
              { value: "Inter", label: "Inter" },
              { value: "system-ui", label: "The system font" },
            ],
            list: "interface-fonts",
          },
        },
      },
    },
    appearance: {
      type: "object",
      properties: {
        accent: {
          type: ["string", "null"],
          "x-lume-format": "accent",
          "x-lume-settings": {
            page: "appearance",
            group: "Theme",
            order: 40,
            label: "Accent colour",
            control: "swatch",
            unset: "Theme\u2019s",
            swatches: [
              { value: "mint", label: "Mint", dark: "#7fe3b8", light: "#147a55" },
              { value: "blue", label: "Blue", dark: "#7aa7ff", light: "#2f6fd0" },
            ],
          },
        },
        "background-image": {
          type: ["string", "null"],
          "x-lume-settings": {
            page: "appearance",
            group: "Panes",
            order: 80,
            label: "Picture",
            help: "The path of an image file.",
            control: "text",
            unset: "None",
          },
        },
        "card-radius": {
          type: ["number", "null"],
          minimum: 0,
          maximum: 64,
          "x-lume-settings": {
            page: "appearance",
            group: "Panes",
            order: 40,
            label: "Pane corners",
            control: "number",
            unset: "Theme\u2019s",
            token: "radius.card",
            unit: "px",
            step: 1,
          },
        },
        "dim-unfocused": {
          type: ["number", "null"],
          minimum: 0,
          maximum: 1,
          "x-lume-settings": {
            page: "appearance",
            group: "Panes",
            order: 50,
            label: "Dim unfocused panes",
            control: "number",
            unset: "Theme\u2019s",
            token: "opacity.dim",
            unit: "%",
            step: 1,
            scale: 100,
          },
        },
        "theme-dark": {
          type: "string",
          default: "lume-dark",
          "x-lume-settings": {
            page: "appearance",
            group: "Theme",
            order: 20,
            label: "In dark mode",
            control: "select",
            list: "themes",
            when: { path: "appearance.theme", is: "unset" },
          },
        },
        theme: {
          type: ["string", "null"],
          "x-lume-settings": {
            page: "appearance",
            group: "Theme",
            order: 10,
            label: "Theme",
            help: "Colours for every window.",
            control: "select",
            list: "themes",
            unset: "Follow system",
          },
        },
        tokens: { type: "object", additionalProperties: true },
      },
    },
  },
};

/** What a machine has to choose from. */
export const LISTS: SettingsLists = {
  themes: [
    { id: "lume-dark", name: "Lume Dark", scheme: "dark", builtin: true },
    { id: "lume-light", name: "Lume Light", scheme: "light", builtin: true },
    { id: "ember", name: "Ember", scheme: "dark", builtin: false },
  ],
  fonts: ["Cascadia Mono", "Consolas"],
  interfaceFonts: ["Cascadia Mono", "Consolas", "Inter", "Segoe UI"],
  shells: [{ name: "PowerShell", program: "pwsh.exe" }],
  workspaces: ["default", "notes"],
  rexHosts: [DEVBOX, { ...NAS, id: "rex:?broken", label: "broken", address: "not an address" }],
  profiles: [
    {
      id: "box",
      name: "Box",
      launcher: { type: "ssh", host: "box.example" },
      hidden: false,
      origin: "user",
    },
    {
      id: "auto.wsl.Ubuntu",
      name: "Ubuntu",
      launcher: { type: "wsl", distro: "Ubuntu" },
      hidden: false,
      origin: "generated",
    },
    {
      id: "auto.cmd",
      name: "Command Prompt",
      launcher: { type: "shell", program: "cmd.exe", args: [], login: false },
      hidden: true,
      origin: "generated",
    },
  ],
};

/**
 * A configuration in use: `changes` replace parts of the view, and `appearance` adds
 * settings to that section.
 */
export function view(changes: Partial<ConfigView> = {}, appearance: Record<string, unknown> = {}) {
  const made: ConfigView = {
    revision: 1,
    platform: "windows",
    scheme: "dark",
    theme: { id: "lume-dark", scheme: "dark" },
    config: settings({
      appearance: { "pane-header": "always", "theme-dark": "lume-dark", ...appearance },
      terminal: { "scrollback-lines": 10_000, "cursor-style": "bar", "cursor-blink": false },
      layout: { "tab-placement": "top" },
      renderer: { tier: "auto" },
      font: { family: "JetBrains Mono" },
    }),
    sources: {},
    problems: [],
    tokens: DEFAULT_TOKENS,
    keymap: { bindings: [], removed: [] },
    schema_revision: 1,
    title_bar: "custom",
    ...changes,
  };
  return made;
}
