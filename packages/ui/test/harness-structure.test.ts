import { describe, expect, it } from "vitest";

import { audit, roleOf, type AuditNode } from "../src/harness/audit.ts";
import {
  describePage,
  differences,
  plainColor,
  recordText,
  type Measured,
  type Part,
} from "../src/harness/structure.ts";

/** An element as the checks read one, with where it is. */
interface Fake extends AuditNode {
  readonly children: Fake[];
  readonly rect: Measured;
}

interface Made {
  readonly attributes?: Readonly<Record<string, string>>;
  readonly text?: string;
  readonly tabIndex?: number;
  readonly rect?: readonly [number, number, number, number];
}

/** Makes an element. A button, a field and a link take the keyboard unless told not to. */
function el(tag: string, made: Made = {}, children: Fake[] = []): Fake {
  const attributes = made.attributes ?? {};
  const tagName = tag.toUpperCase();
  const native = ["BUTTON", "INPUT", "SELECT", "TEXTAREA", "A"].includes(tagName);
  const [x, y, width, height] = made.rect ?? [0, 0, 0, 0];
  return {
    tagName,
    id: attributes["id"] ?? "",
    tabIndex: made.tabIndex ?? (native ? 0 : -1),
    textContent: (made.text ?? "") + children.map((child) => child.textContent ?? "").join(""),
    children,
    rect: { x, y, width, height },
    getAttribute: (name) => attributes[name] ?? null,
  };
}

const measure = (node: AuditNode): Measured => (node as Fake).rect;

describe("the role an element has", () => {
  it("is the one it claims, or the one its kind gives it", () => {
    expect(roleOf(el("div", { attributes: { role: "tab" } }))).toBe("tab");
    expect(roleOf(el("button"))).toBe("button");
    expect(roleOf(el("select"))).toBe("combobox");
    expect(roleOf(el("input"))).toBe("textbox");
    expect(roleOf(el("input", { attributes: { type: "range" } }))).toBe("slider");
    expect(roleOf(el("input", { attributes: { type: "hidden" } }))).toBeNull();
    expect(roleOf(el("a"))).toBeNull();
    expect(roleOf(el("a", { attributes: { href: "#" } }))).toBe("link");
    expect(roleOf(el("div"))).toBeNull();
  });
});

describe("the check for keyboards and screen readers", () => {
  it("finds nothing wrong with controls built to the rules", () => {
    const page = el("div", {}, [
      el("div", { attributes: { role: "tablist", "aria-label": "Tabs" } }, [
        el("button", { attributes: { role: "tab", "aria-selected": "true" }, text: "one" }),
        el("button", {
          attributes: { role: "tab", "aria-selected": "false" },
          text: "two",
          tabIndex: -1,
        }),
      ]),
      el("button", { attributes: { "aria-label": "Close pane" }, tabIndex: -1 }),
      el("label", { attributes: { for: "size" }, text: "Size" }),
      el("select", { attributes: { id: "size" } }),
      el("label", {}, [el("input")]),
      el("div", {
        attributes: {
          role: "separator",
          "aria-label": "Resize columns",
          "aria-valuenow": "50",
        },
        tabIndex: 0,
      }),
      el("div", { attributes: { role: "dialog", "aria-label": "Command palette" } }, [
        el("input", {
          attributes: {
            role: "combobox",
            "aria-label": "Command",
            "aria-expanded": "true",
            "aria-controls": "results",
          },
        }),
        el("div", { attributes: { id: "results", role: "listbox" } }, [
          el("div", { attributes: { role: "option", "aria-selected": "true" }, text: "New Tab" }),
        ]),
      ]),
      // An icon is hidden from a screen reader and takes no keyboard.
      el("svg", { attributes: { "aria-hidden": "true" } }),
    ]);
    expect(audit(page)).toEqual([]);
  });

  it("finds a control with no name", () => {
    const found = audit(el("div", {}, [el("button", { attributes: { class: "pane-button" } })]));
    expect(found).toEqual(["button.pane-button has no name"]);
    expect(audit(el("div", {}, [el("input")]))).toEqual(["input has no name"]);
    // A title names a control too, and so does what is written in a button.
    expect(audit(el("button", { attributes: { title: "Close" } }))).toEqual([]);
    expect(audit(el("button", { text: "Close" }))).toEqual([]);
  });

  it("finds a role without what the role requires or outside where it belongs", () => {
    const found = audit(el("div", {}, [el("div", { attributes: { role: "tab" }, text: "one" })]));
    expect(found).toEqual([
      "div[role=tab] does not say aria-selected",
      "div[role=tab] is not inside a tablist",
    ]);
    const unnamed = audit(el("div", { attributes: { role: "dialog" } }));
    expect(unnamed).toEqual(["div[role=dialog] has no name"]);
    const separator = audit(el("div", { attributes: { role: "separator" }, tabIndex: 0 }));
    expect(separator).toHaveLength(2);
    // A separator the keyboard cannot reach has nothing to say.
    expect(audit(el("div", { attributes: { role: "separator" } }))).toEqual([]);
  });

  it("finds a group the Tab key stops in more than once, or not at all", () => {
    const tab = (tabIndex: number): Fake => {
      return el("button", {
        attributes: { role: "tab", "aria-selected": "false" },
        text: "t",
        tabIndex,
      });
    };
    const list = (tabs: Fake[]): Fake => el("div", { attributes: { role: "tablist" } }, tabs);
    expect(audit(list([tab(0), tab(0)]))).toHaveLength(1);
    expect(audit(list([tab(-1), tab(-1)]))).toHaveLength(1);
    expect(audit(list([tab(0), tab(-1)]))).toEqual([]);
    expect(audit(list([]))).toEqual([]);
  });

  it("finds what jumps the Tab order, what is hidden and focusable, and a missing id", () => {
    expect(audit(el("button", { text: "x", tabIndex: 3 }))).toHaveLength(1);
    const hidden = el("div", { attributes: { "aria-hidden": "true" } }, [
      el("button", { text: "x" }),
    ]);
    expect(audit(hidden)).toEqual([
      "button is hidden from a screen reader and can still take the keyboard",
    ]);
    const dangling = el("input", {
      attributes: { "aria-label": "Find", "aria-controls": "nowhere" },
    });
    expect(audit(dangling)).toEqual([
      'input "Find" names an element called nowhere in aria-controls, and there is none',
    ]);
  });
});

/** A small window: a bar, and two panes with a divider between them. */
function page(dividerAt = 630): Fake {
  return el("div", { attributes: { class: "shell" }, rect: [10, 20, 1280, 800] }, [
    el("div", { attributes: { class: "top-bar" }, rect: [10, 20, 1280, 44] }, [
      el("div", { attributes: { role: "tablist", "aria-label": "Tabs" } }, [
        el("button", { attributes: { role: "tab", "aria-selected": "true" }, text: " one \n" }),
      ]),
      el("svg", { attributes: { "aria-hidden": "true" } }, [el("button", { text: "unseen" })]),
    ]),
    // A wrapper that is nothing itself: its parts count as the window's.
    el("div", { attributes: { class: "wrapper" } }, [
      el(
        "section",
        {
          attributes: { class: "pane-card", "aria-label": "Pane p1", "data-focused": "true" },
          rect: [18.4, 72, 612, 740],
        },
        [el("button", { attributes: { "aria-label": "Close pane" }, tabIndex: -1 })],
      ),
      el("div", {
        attributes: { class: "divider", role: "separator", "aria-orientation": "vertical" },
        rect: [dividerAt + 10, 72, 8, 740],
      }),
    ]),
  ]);
}

describe("a page written down", () => {
  it("keeps what has a role or is measured, in order, and drops the rest", () => {
    expect(describePage(page(), measure, { x: 10, y: 20 })).toEqual({
      box: "shell",
      at: [0, 0, 1280, 800],
      parts: [
        {
          box: "top-bar",
          at: [0, 0, 1280, 44],
          parts: [
            {
              role: "tablist",
              name: "Tabs",
              parts: [{ role: "tab", name: "one", state: { selected: "true" } }],
            },
          ],
        },
        {
          role: "region",
          box: "pane-card",
          name: "Pane p1",
          state: { focused: "true" },
          at: [8, 52, 612, 740],
          parts: [{ role: "button", name: "Close pane" }],
        },
        {
          role: "separator",
          box: "divider",
          state: { orientation: "vertical" },
          at: [630, 52, 8, 740],
        },
      ],
    });
  });

  it("is the same as itself and as a page a pixel off", () => {
    const written = describePage(page(), measure, { x: 10, y: 20 });
    expect(differences(written, written, 0)).toEqual([]);
    const nudged = describePage(page(631), measure, { x: 10, y: 20 });
    expect(differences(written, nudged, 1)).toEqual([]);
    expect(differences(written, nudged, 0)).toEqual([
      "shell › separator was at 630,52,8,740 and is at 631,52,8,740",
    ]);
  });

  it("says where a name, a state or a part differs", () => {
    const written = describePage(page(), measure, { x: 10, y: 20 });
    const tab: Part = { role: "tab", name: "two", state: { selected: "false" } };
    const bar = written.parts?.[0];
    const changed: Part = {
      ...written,
      parts: [
        { ...bar, parts: [{ role: "tablist", name: "Tabs", parts: [tab] }] },
        ...(written.parts ?? []).slice(1),
      ],
    };
    expect(differences(written, changed, 1)).toEqual([
      'shell › top-bar › tablist "Tabs" › tab "one" is now called "two"',
      'shell › top-bar › tablist "Tabs" › tab "one": selected was true and is false',
    ]);
    const fewer: Part = { ...written, parts: (written.parts ?? []).slice(0, 2) };
    expect(differences(written, fewer, 1)).toEqual(["shell had 3 parts and has 2"]);
    const other: Part = { ...written, box: "tab-area" };
    expect(differences(written, other, 1)).toEqual(["tab-area is where shell was"]);
  });
});

describe("a colour as a page reports it", () => {
  it("is written one way whatever way it was reported", () => {
    expect(plainColor("rgb(14, 16, 21)")).toBe("#0e1015");
    expect(plainColor("rgb(14 16 21)")).toBe("#0e1015");
    expect(plainColor("#0E1015")).toBe("#0e1015");
    expect(plainColor("#0e1015ff")).toBe("#0e1015");
    expect(plainColor("#abc")).toBe("#aabbcc");
    expect(plainColor("rgba(240, 179, 90, 0.4)")).toBe("#f0b35a66");
    expect(plainColor("rgb(240 179 90 / 40%)")).toBe("#f0b35a66");
    expect(plainColor("rgba(0, 0, 0, 0)")).toBe("#00000000");
  });

  it("is `mixed` when it was worked out from two others", () => {
    expect(plainColor("color(srgb 0.94 0.7 0.35 / 0.09)")).toBe("mixed");
    expect(plainColor("oklab(0.5 0.1 0.1)")).toBe("mixed");
  });
});

describe("a record as the text of a file", () => {
  it("is indented JSON with a short list of numbers on one line", () => {
    const text = recordText({
      structure: { box: "shell", at: [0, 0, 1280, 800], parts: [{ role: "tab" }], none: [] },
      skipped: undefined,
    });
    expect(text).toBe(
      [
        "{",
        '  "structure": {',
        '    "box": "shell",',
        '    "at": [0, 0, 1280, 800],',
        '    "parts": [',
        "      {",
        '        "role": "tab"',
        "      }",
        "    ],",
        '    "none": []',
        "  }",
        "}",
        "",
      ].join("\n"),
    );
    expect(JSON.parse(text)).toEqual({
      structure: { box: "shell", at: [0, 0, 1280, 800], parts: [{ role: "tab" }], none: [] },
    });
  });
});
