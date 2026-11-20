import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import {
  Badge,
  Button,
  Checkbox,
  clampShare,
  createView,
  flattenTree,
  List,
  Menu,
  type PortLike,
  Segmented,
  SplitButton,
  Switch,
  Tabs,
  Tree,
  type TreeNode,
  useTheme,
  useViewState,
  ViewProvider,
  visibleRange,
} from "../src/index.ts";

const nothing = () => undefined;

describe("the controls", () => {
  it("are the plain controls they look like", () => {
    expect(renderToStaticMarkup(<Button variant="primary">Commit</Button>)).toBe(
      '<button type="button" class="lk-button lk-button-primary">Commit</button>',
    );
    expect(renderToStaticMarkup(<Switch checked label="Wrap" onChange={nothing} />)).toContain(
      'role="switch" aria-checked="true" aria-label="Wrap"',
    );
    expect(renderToStaticMarkup(<Badge tone="info">Pending</Badge>)).toBe(
      '<span class="lk-badge lk-tone-info">Pending</span>',
    );
  });

  it("show a checkbox in each of its three states", () => {
    const box = (checked: boolean | "mixed") =>
      renderToStaticMarkup(<Checkbox checked={checked} label="Staged" onChange={nothing} />);
    expect(box(true)).toContain('aria-checked="true"');
    expect(box(false)).toContain('aria-checked="false"');
    expect(box("mixed")).toContain('aria-checked="mixed"');
    expect(box(false)).not.toContain("<svg");
    expect(box("mixed")).toContain("<svg");
  });

  it("mark the taken choice and the shown tab, and count beside a tab", () => {
    const segmented = renderToStaticMarkup(
      <Segmented
        label="Layout"
        value="split"
        onChange={nothing}
        options={[
          { value: "stacked", label: "Stacked" },
          { value: "split", label: "Split" },
        ]}
      />,
    );
    expect(segmented).toContain('aria-checked="true" tabindex="0">Split');
    expect(segmented).toContain('aria-checked="false" tabindex="-1">Stacked');
    const tabs = renderToStaticMarkup(
      <Tabs
        label="Git"
        value="changes"
        onChange={nothing}
        tabs={[
          { id: "changes", label: "Changes", count: 4 },
          { id: "history", label: "History" },
        ]}
      />,
    );
    expect(tabs).toContain('aria-selected="true"');
    expect(tabs).toContain("(4)");
  });

  it("draw a menu's rows, and a button with its other ways closed", () => {
    const menu = renderToStaticMarkup(
      <Menu
        label="Commit"
        onSelect={nothing}
        items={[
          { id: "push", label: "Commit and push" },
          { separator: true },
          { id: "undo", label: "Undo last commit", danger: true, disabled: true },
        ]}
      />,
    );
    expect(menu.match(/role="menuitem"/g)).toHaveLength(2);
    expect(menu).toContain('role="separator"');
    expect(menu).toContain("disabled");
    const split = renderToStaticMarkup(
      <SplitButton menuLabel="More ways to commit" onSelect={nothing} items={[]}>
        Commit
      </SplitButton>,
    );
    expect(split).toContain('aria-expanded="false"');
    expect(split).not.toContain('role="menu"');
  });
});

describe("rows", () => {
  it("are drawn with the selected one marked", () => {
    const list = renderToStaticMarkup(
      <List
        label="Files"
        items={["a.rs", "b.rs"]}
        getKey={(item) => item}
        selected="b.rs"
        renderRow={(item, state) => `${state.index}:${item}`}
      />,
    );
    expect(list).toContain('aria-selected="true" class="lk-row">1:b.rs');
    expect(list).toContain('aria-selected="false" class="lk-row">0:a.rs');
  });

  it("are only those in sight in a long list", () => {
    expect(visibleRange(0, 280, 28, 1000, 0)).toEqual({ first: 0, end: 10 });
    expect(visibleRange(2800, 280, 28, 1000, 2)).toEqual({ first: 98, end: 112 });
    expect(visibleRange(999999, 280, 28, 50, 6)).toEqual({ first: 50, end: 50 });
    expect(visibleRange(0, 280, 28, 0)).toEqual({ first: 0, end: 0 });
    expect(visibleRange(0, 280, 28, 3)).toEqual({ first: 0, end: 3 });
  });

  const nodes: TreeNode<string>[] = [
    {
      id: "src",
      data: "src",
      children: [
        { id: "src/a.rs", data: "a.rs" },
        { id: "src/sub", data: "sub", children: [{ id: "src/sub/b.rs", data: "b.rs" }] },
      ],
    },
    { id: "readme", data: "README.md" },
  ];

  it("of a tree are those under open nodes", () => {
    const ids = (open: string[]) => flattenTree(nodes, new Set(open)).map((one) => one.node.id);
    expect(ids([])).toEqual(["src", "readme"]);
    expect(ids(["src"])).toEqual(["src", "src/a.rs", "src/sub", "readme"]);
    expect(ids(["src", "src/sub"])).toEqual([
      "src",
      "src/a.rs",
      "src/sub",
      "src/sub/b.rs",
      "readme",
    ]);
    expect(flattenTree(nodes, new Set(["src"]))[1]).toMatchObject({ depth: 1, parent: "src" });
  });

  it("of a tree say how deep they are and whether they are open", () => {
    const tree = renderToStaticMarkup(
      <Tree
        label="Files"
        nodes={nodes}
        expanded={new Set(["src"])}
        onToggle={nothing}
        renderRow={(data, state) => `${data}@${state.depth}${state.leaf ? "" : "/"}`}
      />,
    );
    expect(tree).toContain('role="tree"');
    expect(tree).toContain('aria-level="2"');
    expect(tree).toContain('aria-expanded="true"');
    expect(tree).toContain("a.rs@1");
    expect(tree).toContain("sub@1/");
    expect(tree).not.toContain("b.rs");
  });

  it("share their room within bounds", () => {
    expect(clampShare(0.5, 0.15, 0.85)).toBe(0.5);
    expect(clampShare(-1, 0.15, 0.85)).toBe(0.15);
    expect(clampShare(2, 0.15, 0.85)).toBe(0.85);
    expect(clampShare(Number.NaN, 0.15, 0.85)).toBe(0.15);
  });
});

describe("the hooks", () => {
  it("read the view a provider holds", () => {
    const port: PortLike = { onmessage: null, postMessage: nothing };
    const view = createView(
      {
        lume: "connect",
        api: 1,
        instance: "main/v1",
        plugin: "acme.notes",
        view: "notes",
        place: "panel",
        state: { tab: "history" },
        tokens: {},
        scheme: "light",
      },
      port,
    );
    function Shown() {
      const [state] = useViewState({ tab: "changes" });
      const theme = useTheme();
      return (
        <p>
          {state.tab} {theme.scheme}
        </p>
      );
    }
    expect(
      renderToStaticMarkup(
        <ViewProvider view={view}>
          <Shown />
        </ViewProvider>,
      ),
    ).toBe("<p>history light</p>");
    expect(() => renderToStaticMarkup(<Shown />)).toThrow("ViewProvider");
  });
});
