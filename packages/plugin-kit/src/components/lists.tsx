/**
 * Rows: a list, a list that draws only the rows in sight, and a tree.
 *
 * What a row shows is the view's to draw; these keep which row is selected, move the
 * selection with the arrow keys, and scroll. A long list, a file's history or a
 * repository's files, is a `VirtualList` or a `Tree` with a `rowHeight`: its rows are
 * all the same height, and only those in sight exist.
 */

import {
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { ChevronRight, cx } from "./icons";

/** Which rows of `count`, each `rowHeight` tall, are in sight, with `overscan` spare. */
export function visibleRange(
  scrollTop: number,
  viewport: number,
  rowHeight: number,
  count: number,
  overscan = 6,
): { readonly first: number; readonly end: number } {
  if (count <= 0 || rowHeight <= 0) return { first: 0, end: 0 };
  const first = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const end = Math.min(count, Math.ceil((scrollTop + viewport) / rowHeight) + overscan);
  return { first: Math.min(first, count), end: Math.max(end, Math.min(first, count)) };
}

/** What a row is told of itself. */
export interface RowState {
  readonly index: number;
  readonly selected: boolean;
}

interface RowsProps<T> {
  readonly items: readonly T[];
  readonly getKey: (item: T, index: number) => string;
  readonly renderRow: (item: T, state: RowState) => ReactNode;
  readonly selected?: string | null | undefined;
  readonly onSelect?: ((key: string, item: T) => void) | undefined;
  /** Enter or a double click on a row. */
  readonly onActivate?: ((key: string, item: T) => void) | undefined;
  /** Other keys pressed with a row selected; return `true` where the key was used. */
  readonly onRowKey?: ((event: KeyboardEvent, item: T) => boolean) | undefined;
  readonly label: string;
  readonly className?: string | undefined;
  readonly role?: "listbox" | "tree" | undefined;
  readonly rowProps?: ((item: T, index: number) => Record<string, unknown>) | undefined;
  /** Every row's height in pixels: only the rows in sight are drawn. */
  readonly rowHeight?: number | undefined;
}

function Rows<T>({
  items,
  getKey,
  renderRow,
  selected,
  onSelect,
  onActivate,
  onRowKey,
  label,
  className,
  role = "listbox",
  rowProps,
  rowHeight,
}: RowsProps<T>) {
  const box = useRef<HTMLDivElement>(null);
  const [scroll, setScroll] = useState({ top: 0, height: 0 });
  const virtual = rowHeight !== undefined;

  useLayoutEffect(() => {
    const element = box.current;
    if (!virtual || !element) return undefined;
    const measure = () =>
      setScroll((now) =>
        now.height === element.clientHeight ? now : { ...now, height: element.clientHeight },
      );
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [virtual]);

  const keys = useMemo(() => items.map(getKey), [items, getKey]);
  const at = selected == null ? -1 : keys.indexOf(selected);

  // The selected row is brought into sight when the selection moves.
  useEffect(() => {
    const element = box.current;
    if (!element || at < 0) return;
    if (rowHeight !== undefined) {
      const top = at * rowHeight;
      if (top < element.scrollTop) element.scrollTop = top;
      else if (top + rowHeight > element.scrollTop + element.clientHeight) {
        element.scrollTop = top + rowHeight - element.clientHeight;
      }
    } else {
      element.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
    }
  }, [at, rowHeight]);

  const select = useCallback(
    (index: number) => {
      const item = items[index];
      const key = keys[index];
      if (item !== undefined && key !== undefined) onSelect?.(key, item);
    },
    [items, keys, onSelect],
  );

  const keyDown = (event: KeyboardEvent) => {
    const item = at >= 0 ? items[at] : undefined;
    if (item !== undefined && onRowKey?.(event, item) === true) {
      event.preventDefault();
      return;
    }
    const page = Math.max(1, Math.floor(scroll.height / (rowHeight ?? 28)) - 1);
    let next: number;
    switch (event.key) {
      case "ArrowDown":
        next = Math.min(items.length - 1, at + 1);
        break;
      case "ArrowUp":
        next = Math.max(0, at - 1);
        break;
      case "PageDown":
        next = Math.min(items.length - 1, at + page);
        break;
      case "PageUp":
        next = Math.max(0, at - page);
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = items.length - 1;
        break;
      case "Enter":
        if (item !== undefined && selected != null) {
          onActivate?.(selected, item);
          event.preventDefault();
        }
        return;
      default:
        return;
    }
    event.preventDefault();
    if (next !== at) select(next);
  };

  const range = virtual
    ? visibleRange(scroll.top, scroll.height, rowHeight, items.length)
    : { first: 0, end: items.length };
  const rows = [];
  for (let index = range.first; index < range.end; index++) {
    const item = items[index] as T;
    const key = keys[index] as string;
    const isSelected = key === selected;
    rows.push(
      <div
        key={key}
        role={role === "tree" ? "treeitem" : "option"}
        aria-selected={isSelected}
        className="lk-row"
        style={virtual ? { height: rowHeight } : undefined}
        onClick={() => select(index)}
        onDoubleClick={() => onActivate?.(key, item)}
        {...rowProps?.(item, index)}
      >
        {renderRow(item, { index, selected: isSelected })}
      </div>,
    );
  }

  return (
    <div
      ref={box}
      role={role}
      aria-label={label}
      tabIndex={0}
      className={cx("lk-list", "lk-scroll", className)}
      onKeyDown={keyDown}
      onScroll={
        virtual
          ? (event) => {
              const top = event.currentTarget.scrollTop;
              setScroll((now) => (now.top === top ? now : { ...now, top }));
            }
          : undefined
      }
    >
      {virtual ? (
        <div className="lk-list-space" style={{ height: items.length * rowHeight }}>
          <div style={{ transform: `translateY(${range.first * rowHeight}px)` }}>{rows}</div>
        </div>
      ) : (
        rows
      )}
    </div>
  );
}

export interface ListProps<T> {
  readonly items: readonly T[];
  /** A name for each row that stays the same while the row does. */
  readonly getKey: (item: T, index: number) => string;
  readonly renderRow: (item: T, state: RowState) => ReactNode;
  /** The selected row's key. */
  readonly selected?: string | null | undefined;
  readonly onSelect?: ((key: string, item: T) => void) | undefined;
  /** Enter or a double click on a row. */
  readonly onActivate?: ((key: string, item: T) => void) | undefined;
  /** What the list is, for assistive technology. */
  readonly label: string;
  readonly className?: string | undefined;
}

/** A list of rows, one of which may be selected. */
export function List<T>(props: ListProps<T>) {
  return <Rows {...props} />;
}

export interface VirtualListProps<T> extends ListProps<T> {
  /** Every row's height, in pixels. */
  readonly rowHeight: number;
}

/**
 * A list that draws only the rows in sight, for thousands of rows of one height. It
 * fills the box it is put in, which must have a height.
 */
export function VirtualList<T>(props: VirtualListProps<T>) {
  return <Rows {...props} />;
}

/** One thing in a tree, with what is under it. */
export interface TreeNode<T> {
  readonly id: string;
  readonly data: T;
  /** What is under it. A node with none, or with an empty list, is a leaf. */
  readonly children?: readonly TreeNode<T>[] | undefined;
}

/** What a tree's row is told of itself. */
export interface TreeRowState extends RowState {
  readonly depth: number;
  readonly expanded: boolean;
  readonly leaf: boolean;
}

interface FlatNode<T> {
  readonly node: TreeNode<T>;
  readonly depth: number;
  readonly parent: string | null;
}

/** The nodes of a tree that are in sight, in order: those under an open node. */
export function flattenTree<T>(
  nodes: readonly TreeNode<T>[],
  expanded: ReadonlySet<string>,
): FlatNode<T>[] {
  const flat: FlatNode<T>[] = [];
  const walk = (list: readonly TreeNode<T>[], depth: number, parent: string | null) => {
    for (const node of list) {
      flat.push({ node, depth, parent });
      if (node.children && node.children.length > 0 && expanded.has(node.id)) {
        walk(node.children, depth + 1, node.id);
      }
    }
  };
  walk(nodes, 0, null);
  return flat;
}

export interface TreeProps<T> {
  readonly nodes: readonly TreeNode<T>[];
  /** The ids of the nodes that are open. */
  readonly expanded: ReadonlySet<string>;
  /** Called when a node should open or close. */
  readonly onToggle: (id: string, open: boolean) => void;
  /** Draws a node's row, after its indent and its arrow. */
  readonly renderRow: (data: T, state: TreeRowState) => ReactNode;
  readonly selected?: string | null | undefined;
  readonly onSelect?: ((id: string, data: T) => void) | undefined;
  readonly onActivate?: ((id: string, data: T) => void) | undefined;
  readonly label: string;
  readonly className?: string | undefined;
  /** Every row's height in pixels; given, only the rows in sight are drawn. */
  readonly rowHeight?: number | undefined;
  /** How far each level is indented, in pixels. */
  readonly indent?: number | undefined;
}

/**
 * A tree: folders and what is in them. Which nodes are open is the view's to keep.
 * Right opens a node or goes into it, Left closes it or goes to its parent.
 */
export function Tree<T>({
  nodes,
  expanded,
  onToggle,
  renderRow,
  selected,
  onSelect,
  onActivate,
  label,
  className,
  rowHeight,
  indent = 14,
}: TreeProps<T>) {
  const flat = useMemo(() => flattenTree(nodes, expanded), [nodes, expanded]);
  const getKey = useCallback((item: FlatNode<T>) => item.node.id, []);
  const isLeaf = (item: FlatNode<T>) => !item.node.children || item.node.children.length === 0;
  return (
    <Rows
      items={flat}
      getKey={getKey}
      role="tree"
      label={label}
      className={cx("lk-tree", className)}
      rowHeight={rowHeight}
      selected={selected}
      onSelect={(id, item) => onSelect?.(id, item.node.data)}
      onActivate={(id, item) => {
        if (isLeaf(item)) onActivate?.(id, item.node.data);
        else onToggle(id, !expanded.has(id));
      }}
      rowProps={(item) => ({
        "aria-level": item.depth + 1,
        "aria-expanded": isLeaf(item) ? undefined : expanded.has(item.node.id),
      })}
      onRowKey={(event, item) => {
        const open = expanded.has(item.node.id);
        if (event.key === "ArrowRight" && !isLeaf(item)) {
          if (!open) onToggle(item.node.id, true);
          else {
            const child = item.node.children?.[0];
            if (child) onSelect?.(child.id, child.data);
          }
          return true;
        }
        if (event.key === "ArrowLeft") {
          if (!isLeaf(item) && open) onToggle(item.node.id, false);
          else if (item.parent !== null) {
            const parent = flat.find((other) => other.node.id === item.parent);
            if (parent) onSelect?.(parent.node.id, parent.node.data);
          }
          return true;
        }
        return false;
      }}
      renderRow={(item, state) => {
        const leaf = isLeaf(item);
        const open = expanded.has(item.node.id);
        return (
          <>
            <span className="lk-tree-indent" style={{ width: item.depth * indent }} />
            {leaf ? (
              <span className="lk-tree-arrow" />
            ) : (
              <span
                className={cx("lk-tree-arrow", open && "lk-open")}
                onClick={(event) => {
                  event.stopPropagation();
                  onToggle(item.node.id, !open);
                }}
              >
                <ChevronRight size={11} />
              </span>
            )}
            {renderRow(item.node.data, { ...state, depth: item.depth, expanded: open, leaf })}
          </>
        );
      }}
    />
  );
}
