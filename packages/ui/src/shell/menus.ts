/**
 * A menu's rows, from the menu's nodes as the core placed them.
 *
 * The core puts each menu together: the application's own sections and items, and the
 * plugins' beside them in the places their manifests ask for. A menu here is drawn
 * from that, in its order. Each of the application's own nodes is drawn by code of the
 * menu's own, found by the node's key; a plugin's item is a row that runs its command,
 * and a plugin's submenu a row that opens its items. Sections are set apart by lines,
 * and a plugin's section with a title has it as a heading. A node whose conditions do
 * not hold is left out, and so is a section left with nothing in it.
 *
 * Before the core has said, a menu is drawn from the application's own nodes alone,
 * which are generated from the same places the core works from.
 */

import type { MenuNodeData } from "../core/types.ts";
import { holds } from "../plugins/store.ts";
import type { MenuItem } from "./Menu.tsx";
import { BUILTIN_PLACES, type BuiltinNode, type MENUS } from "./places.generated.ts";

/** A menu's name. */
export type MenuName = (typeof MENUS)[number];

/** What drawing a menu's rows needs. */
export interface MenuParts {
  /** The rows one of the application's nodes stands for, by its key without `lume:`. */
  builtin(key: string): readonly MenuItem[];
  /** A row for a plugin's item: its label, picture and what choosing it does. */
  plugin(node: MenuNodeData): MenuItem | undefined;
  /** The conditions that hold now. */
  readonly contexts: ReadonlySet<string>;
}

/** The application's own nodes of a menu, as the core would send them with no plugins. */
function builtinNodes(nodes: readonly BuiltinNode[]): MenuNodeData[] {
  return nodes.map((node) => ({
    kind: node.kind,
    key: `lume:${node.key}`,
    ...(node.children.length === 0 ? {} : { children: builtinNodes(node.children) }),
  }));
}

/** A menu's nodes: what the core sent, or the application's own while it has sent none. */
export function nodesOf(
  menus: Readonly<Record<string, readonly MenuNodeData[]>> | undefined,
  menu: MenuName,
): readonly MenuNodeData[] {
  return menus?.[menu] ?? builtinNodes(BUILTIN_PLACES[menu]);
}

/** The rows of one node that is not a section. */
function rowsOf(node: MenuNodeData, parts: MenuParts): readonly MenuItem[] {
  if (!holds(node.when, parts.contexts)) return [];
  if (node.plugin === undefined) {
    return parts.builtin(node.key.startsWith("lume:") ? node.key.slice(5) : node.key);
  }
  if (node.kind === "submenu") {
    const items = menuRows(node.children ?? [], parts);
    if (items.length === 0) return [];
    return [{ key: node.key, label: node.title ?? node.key, items, choose: () => undefined }];
  }
  const row = parts.plugin(node);
  return row ? [row] : [];
}

/**
 * A menu's rows from its nodes. A section is set apart from what comes before and
 * after it; items that are in no section run together.
 */
export function menuRows(nodes: readonly MenuNodeData[], parts: MenuParts): MenuItem[] {
  const rows: MenuItem[] = [];
  let afterSection = false;
  for (const node of nodes) {
    let group: MenuItem[];
    if (node.kind === "section") {
      if (!holds(node.when, parts.contexts)) continue;
      group = (node.children ?? []).flatMap((child) => rowsOf(child, parts));
      if (group.length > 0 && node.plugin !== undefined && node.title !== undefined) {
        group.unshift({
          key: `heading:${node.key}`,
          kind: "heading",
          label: node.title,
          choose: () => undefined,
        });
      }
    } else {
      group = [...rowsOf(node, parts)];
    }
    if (group.length === 0) continue;
    const [first, ...rest] = group;
    if (first === undefined) continue;
    const section = node.kind === "section";
    const apart = rows.length > 0 && (section || afterSection);
    rows.push(apart ? { ...first, separated: true } : first, ...rest);
    afterSection = section;
  }
  return rows;
}
