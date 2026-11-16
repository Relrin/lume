import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { pageInAddress } from "../src/settings/boot.ts";
import type { SettingRow } from "../src/settings/model.ts";
import { Button, Row } from "../src/settings/rows.tsx";

function row(changes: Partial<SettingRow>): SettingRow {
  return {
    path: "terminal.cursor-style",
    label: "Cursor style",
    control: "segmented",
    choices: [],
    value: null,
    effective: null,
    source: "default",
    overridden: false,
    shown: true,
    ...changes,
  };
}

function markup(held: SettingRow, more: { failure?: string; open?: () => void } = {}): string {
  return renderToStaticMarkup(createElement(Row, { row: held, write: () => undefined, ...more }));
}

/** The id the row's name has, which its control is named by. */
function labelId(html: string): string {
  const found = /<div id="([^"]+)">/.exec(html);
  if (!found?.[1]) throw new Error("the row has no name to be labelled by");
  return found[1];
}

describe("a setting drawn as a row", () => {
  it("names its control after the row, and says what is said of the setting", () => {
    const html = markup(
      row({ control: "switch", label: "Blinking cursor", help: "Until a program says otherwise." }),
    );
    expect(html).toContain(`aria-labelledby="${labelId(html)}"`);
    expect(html).toContain(">Blinking cursor</div>");
    expect(html).toContain("<div>Until a program says otherwise.</div>");
    expect(html).toContain('data-path="terminal.cursor-style"');
    // Nothing is styled by an attribute of its own.
    expect(html).not.toContain("style=");
  });

  it("draws a switch that says whether it is on", () => {
    expect(markup(row({ control: "switch", effective: true }))).toContain(
      'role="switch" class="settings-switch" aria-checked="true"',
    );
    expect(markup(row({ control: "switch", effective: false }))).toContain('aria-checked="false"');
  });

  it("draws the choices of a select with the one in use chosen", () => {
    const html = markup(
      row({
        control: "select",
        value: 6,
        effective: 6,
        choices: [
          { value: null, label: "Theme\u2019s (10 px)" },
          { value: 6, label: "6 px" },
          { value: 10, label: "10 px" },
        ],
      }),
    );
    expect(html).toContain('<option value="0">Theme\u2019s (10 px)</option>');
    expect(html).toContain('<option value="1" selected="">6 px</option>');
    expect(html).toContain('<option value="2">10 px</option>');
  });

  it("draws segments as buttons, the one in use pressed", () => {
    const html = markup(
      row({
        value: null,
        effective: "bar",
        choices: [
          { value: "block", label: "Block" },
          { value: "bar", label: "Bar" },
        ],
      }),
    );
    expect(html).toContain('role="group"');
    expect(html).toContain('<button type="button" aria-pressed="false">Block</button>');
    expect(html).toContain('<button type="button" aria-pressed="true">Bar</button>');
  });

  it("draws a field holding what is set, or naming what leaving it out means", () => {
    const set = markup(row({ control: "text", value: "C:\\hills.png", unsetLabel: "None" }));
    expect(set).toContain('value="C:\\hills.png"');
    const unset = markup(row({ control: "text", unsetLabel: "None" }));
    expect(unset).toContain('placeholder="None"');
    expect(unset).toContain('value=""');
  });

  it("says when a file other than the user's decides the setting", () => {
    const html = markup(row({ source: "workspace", overridden: true }));
    expect(html).toContain('class="settings-note"');
    expect(html).toContain("workspace");
    expect(markup(row({}))).not.toContain("settings-note");
  });

  it("shows why a change was not made before what is wrong in the file", () => {
    const wrong = markup(row({ problem: "config.toml:7: not a cursor style" }));
    expect(wrong).toContain('role="alert"');
    expect(wrong).toContain("config.toml:7: not a cursor style");
    const failed = markup(row({ problem: "config.toml:7: not a cursor style" }), {
      failure: "The file could not be written",
    });
    expect(failed).toContain("The file could not be written");
    expect(failed).not.toContain("config.toml:7");
    expect(markup(row({}))).not.toContain('role="alert"');
  });

  it("marks a row that is itself pressed, as a search result is", () => {
    expect(markup(row({}), { open: () => undefined })).toContain('data-hit="true"');
    expect(markup(row({}))).toContain('data-hit="false"');
  });

  it("draws a button that says whether it is a quiet one", () => {
    const html = renderToStaticMarkup(
      createElement(Button, {
        quiet: true,
        onPress: () => undefined,
        children: "Open config.toml",
      }),
    );
    expect(html).toBe(
      '<button type="button" class="settings-button" data-quiet="true" data-primary="false">Open config.toml</button>',
    );
  });
});

describe("the page a settings window is opened at", () => {
  it("is the one its address names", () => {
    expect(pageInAddress("#appearance")).toBe("appearance");
    expect(pageInAddress("appearance")).toBe("appearance");
    expect(pageInAddress("#add-on%20one")).toBe("add-on one");
  });

  it("is none when the address names none", () => {
    expect(pageInAddress("")).toBeNull();
    expect(pageInAddress("#")).toBeNull();
    expect(pageInAddress("#  ")).toBeNull();
  });
});
