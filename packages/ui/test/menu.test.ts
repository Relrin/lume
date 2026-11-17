import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { Menu, type MenuItem } from "../src/shell/Menu.tsx";

const nothing = () => undefined;

describe("a menu", () => {
  it("marks an item that opens a menu of its own, and draws an item's picture", () => {
    const items: MenuItem[] = [
      { key: "tab", label: "New tab", hint: "Ctrl T", choose: nothing },
      {
        key: "with",
        label: "New tab with",
        items: [{ key: "a", label: "A", choose: nothing }],
        choose: nothing,
      },
      { key: "pic", label: "With a picture", icon: createElement("i"), choose: nothing },
    ];
    const html = renderToStaticMarkup(
      createElement(Menu, { label: "Lume", items, close: nothing }),
    );
    expect(html).toContain('aria-haspopup="menu" aria-expanded="false" data-key="with"');
    expect(html).toContain('<span class="menu-more">');
    expect(html).toContain('<span class="menu-icon"><i></i></span>');
    // An item without a menu of its own says nothing of one.
    expect(html).toMatch(/role="menuitem" data-key="tab"/);
    // Its own menu is not drawn until it is opened.
    expect(html).not.toContain("menu-sub-place");
    expect(html).toContain('class="menu-backdrop"');
  });

  it("draws no backdrop for a menu opened from an item of another", () => {
    const html = renderToStaticMarkup(
      createElement(Menu, {
        label: "New tab with",
        items: [{ key: "a", label: "A", choose: nothing }],
        close: nothing,
        back: nothing,
      }),
    );
    expect(html).not.toContain("menu-backdrop");
    expect(html).toContain('class="menu menu-sub"');
  });
});
