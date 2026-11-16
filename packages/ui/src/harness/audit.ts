/**
 * A check of what a page offers to someone who does not use a pointer or cannot see
 * the screen: that everything that can be pressed has a name, that what claims a role
 * says what the role requires, and that the keyboard can reach it in a sensible way.
 *
 * It is not a full audit. It holds the page to the rules its own controls are built
 * to, so a control added without them is found by a test, not by a user.
 *
 * The check reads elements through a few of their properties only, so it can be run on
 * a description of a page as well as on a page.
 */

/** As much of an element as the check reads. */
export interface AuditNode {
  readonly tagName: string;
  readonly id: string;
  /** Where the keyboard's Tab key counts the element: nought or more reaches it. */
  readonly tabIndex: number;
  readonly textContent: string | null;
  readonly children: ArrayLike<AuditNode>;
  getAttribute(name: string): string | null;
}

/** The roles an element has without saying so. */
const NATIVE_ROLES: Readonly<Record<string, string>> = {
  BUTTON: "button",
  SELECT: "combobox",
  TEXTAREA: "textbox",
  A: "link",
};

/** The roles whose elements are pressed, chosen or typed into. */
const INTERACTIVE = new Set([
  "button",
  "link",
  "tab",
  "switch",
  "checkbox",
  "radio",
  "menuitem",
  "menuitemradio",
  "menuitemcheckbox",
  "option",
  "combobox",
  "textbox",
  "searchbox",
  "slider",
]);

/** The roles named by what is written in them when nothing else names them. */
const NAMED_BY_CONTENT = new Set([
  "button",
  "link",
  "tab",
  "radio",
  "menuitem",
  "menuitemradio",
  "menuitemcheckbox",
  "option",
]);

/** The attribute each role cannot do without. */
const REQUIRED: Readonly<Record<string, string>> = {
  tab: "aria-selected",
  switch: "aria-checked",
  checkbox: "aria-checked",
  radio: "aria-checked",
  menuitemradio: "aria-checked",
  menuitemcheckbox: "aria-checked",
  option: "aria-selected",
};

/** The role each role must be somewhere inside. */
const INSIDE: Readonly<Record<string, readonly string[]>> = {
  tab: ["tablist"],
  radio: ["radiogroup"],
  option: ["listbox"],
  menuitem: ["menu", "menubar"],
  menuitemradio: ["menu", "menubar"],
  menuitemcheckbox: ["menu", "menubar"],
};

/** The groups the keyboard enters at one member and moves about in with the arrows. */
const ROVING: Readonly<Record<string, string>> = { tablist: "tab", radiogroup: "radio" };

/** The attributes that name other elements by their ids. */
const REFERENCES = [
  "aria-controls",
  "aria-labelledby",
  "aria-describedby",
  "aria-activedescendant",
];

/** The role an element has: the one it claims, or the one its kind gives it. */
export function roleOf(node: AuditNode): string | null {
  const claimed = node.getAttribute("role");
  if (claimed !== null && claimed.trim() !== "") return claimed.trim();
  if (node.tagName === "INPUT") {
    const type = node.getAttribute("type") ?? "text";
    if (type === "hidden") return null;
    if (type === "checkbox") return "checkbox";
    if (type === "radio") return "radio";
    if (type === "range") return "slider";
    if (type === "button" || type === "submit") return "button";
    return "textbox";
  }
  if (node.tagName === "A") return node.getAttribute("href") === null ? null : "link";
  return NATIVE_ROLES[node.tagName] ?? null;
}

/** What an element is called in a report: enough to find it. */
function called(node: AuditNode): string {
  const tag = node.tagName.toLowerCase();
  const role = node.getAttribute("role");
  const classes = (node.getAttribute("class") ?? "").trim().split(/\s+/)[0];
  const label = node.getAttribute("aria-label");
  let name = tag;
  if (classes) name += `.${classes}`;
  if (role) name += `[role=${role}]`;
  if (label) name += ` "${label}"`;
  return name;
}

function isDisabled(node: AuditNode): boolean {
  return node.getAttribute("disabled") !== null || node.getAttribute("aria-disabled") === "true";
}

function text(node: AuditNode): string {
  return (node.textContent ?? "").replace(/\s+/g, " ").trim();
}

/** One element with the elements it is inside of, nearest last. */
interface Found {
  readonly node: AuditNode;
  readonly above: readonly AuditNode[];
}

function everything(root: AuditNode): Found[] {
  const found: Found[] = [];
  const walk = (node: AuditNode, above: readonly AuditNode[]): void => {
    found.push({ node, above });
    const inside = [...above, node];
    for (let index = 0; index < node.children.length; index++) {
      const child = node.children[index];
      if (child) walk(child, inside);
    }
  };
  walk(root, []);
  return found;
}

/**
 * What is wrong under `root`, each as a sentence. Nothing wrong is an empty list.
 *
 * The rules:
 * - whatever can be pressed, chosen or typed into has a name;
 * - a role's required attribute is there, and the role is inside the role it needs;
 * - a dialog has a name;
 * - a separator the keyboard can reach says where it stands;
 * - a group moved about in with the arrows is entered at exactly one member;
 * - nothing is put before everything else in the order of the Tab key;
 * - nothing hidden from a screen reader can take the keyboard;
 * - an element named by id exists.
 */
export function audit(root: AuditNode): string[] {
  const problems: string[] = [];
  const all = everything(root);
  const ids = new Set<string>();
  const labelled = new Set<string>();
  for (const { node } of all) {
    if (node.id !== "") ids.add(node.id);
    if (node.tagName === "LABEL") {
      const target = node.getAttribute("for");
      if (target !== null) labelled.add(target);
    }
  }

  for (const { node, above } of all) {
    const role = roleOf(node);
    const what = called(node);

    if (node.tabIndex > 0) {
      problems.push(`${what} is put before everything else in the order of the Tab key`);
    }

    const hidden = [...above, node].some((part) => part.getAttribute("aria-hidden") === "true");
    if (hidden && node.tabIndex >= 0 && role !== null && INTERACTIVE.has(role)) {
      problems.push(`${what} is hidden from a screen reader and can still take the keyboard`);
    }
    // What a screen reader is not shown is not held to the rest.
    if (hidden) continue;

    for (const attribute of REFERENCES) {
      const named = node.getAttribute(attribute);
      if (named === null) continue;
      for (const id of named.trim().split(/\s+/)) {
        if (id !== "" && !ids.has(id)) {
          problems.push(`${what} names an element called ${id} in ${attribute}, and there is none`);
        }
      }
    }
    if (node.tagName === "LABEL") {
      const target = node.getAttribute("for");
      if (target !== null && !ids.has(target)) {
        problems.push(`a label is for an element called ${target}, and there is none`);
      }
    }

    if (role === null) continue;

    if (INTERACTIVE.has(role)) {
      const inLabel = above.some((part) => part.tagName === "LABEL");
      const named =
        (node.getAttribute("aria-label") ?? "").trim() !== "" ||
        node.getAttribute("aria-labelledby") !== null ||
        (node.getAttribute("title") ?? "").trim() !== "" ||
        (node.id !== "" && labelled.has(node.id)) ||
        inLabel ||
        (NAMED_BY_CONTENT.has(role) && text(node) !== "");
      if (!named) problems.push(`${what} has no name`);
    }

    const required = REQUIRED[role];
    if (required !== undefined && node.getAttribute(required) === null) {
      problems.push(`${what} does not say ${required}`);
    }
    const inside = INSIDE[role];
    if (inside !== undefined) {
      const held = above.some((part) => inside.includes(part.getAttribute("role") ?? ""));
      if (!held) problems.push(`${what} is not inside a ${inside.join(" or a ")}`);
    }

    if (role === "dialog" || role === "alertdialog") {
      const named =
        (node.getAttribute("aria-label") ?? "").trim() !== "" ||
        node.getAttribute("aria-labelledby") !== null;
      if (!named) problems.push(`${what} has no name`);
    }
    if (role === "combobox" && node.tagName !== "SELECT") {
      if (node.getAttribute("aria-expanded") === null) {
        problems.push(`${what} does not say aria-expanded`);
      }
    }
    if (role === "separator" && node.tabIndex >= 0) {
      if (node.getAttribute("aria-valuenow") === null) {
        problems.push(`${what} can take the keyboard and does not say aria-valuenow`);
      }
      if ((node.getAttribute("aria-label") ?? "").trim() === "") {
        problems.push(`${what} can take the keyboard and has no name`);
      }
    }

    const member = ROVING[role];
    if (member !== undefined) {
      const members = everything(node)
        .map((found) => found.node)
        .filter((inner) => roleOf(inner) === member && !isDisabled(inner));
      const reachable = members.filter((inner) => inner.tabIndex >= 0).length;
      if (members.length > 0 && reachable !== 1) {
        problems.push(
          `${what} has ${members.length} members and the Tab key stops at ${reachable} of them, not one`,
        );
      }
    }
  }
  return problems;
}
