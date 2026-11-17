/**
 * The editor of one group of tabs, opened with the other button on its label: its
 * picture, name and colour, and what can be done with the group as a whole.
 *
 * What is chosen here is written as it is chosen; the name as it rests between keys,
 * and when the editor is put away. Escape, Enter in the name, or a press outside put it
 * away.
 */

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useStore } from "zustand";

import type { WorkspaceList } from "../core/types.ts";
import { selectGroup, selectGroupTabs, selectTabOrder, stripItems } from "../layout/selectors.ts";
import { GROUP_ICONS, TAB_COLORS, type GroupId, type GroupIcon } from "../layout/types.ts";
import { GROUP_NAME_WRITE_MS } from "../tuning.ts";
import { AnchoredMenu } from "./AnchoredMenu.tsx";
import { useShell } from "./context.ts";
import { roveWithin } from "./roving.ts";
import { ChevronRightIcon, GroupGlyph, MenuGlyph } from "./icons.tsx";
import { GROUP_ICON_NAMES, TAB_COLOR_NAMES, tabColorValue, tabCount } from "./tab-looks.ts";
import { anchorOf, labelId, type Anchor } from "./tab-ui.ts";

/** How wide the editor is. */
const GROUP_EDITOR_WIDTH = 284;

/** One of the editor's rows: a button that does one thing. */
function Row({
  glyph,
  label,
  hint,
  disabled,
  danger,
  pressed,
  expanded,
  onChoose,
}: {
  glyph: Parameters<typeof MenuGlyph>[0]["name"];
  label: string;
  hint?: string;
  disabled?: boolean;
  danger?: boolean;
  pressed?: boolean;
  expanded?: boolean;
  onChoose: () => void;
}) {
  return (
    <button
      type="button"
      className="menu-item group-editor-row"
      role="menuitem"
      disabled={disabled}
      data-danger={danger || undefined}
      aria-pressed={pressed}
      aria-expanded={expanded}
      onClick={onChoose}
    >
      <span className="menu-icon">
        <MenuGlyph name={glyph} />
      </span>
      <span className="menu-label">{label}</span>
      {hint !== undefined && <span className="menu-hint">{hint}</span>}
      {expanded !== undefined && (
        <span className="menu-more group-editor-chevron">
          <ChevronRightIcon />
        </span>
      )}
    </button>
  );
}

export function GroupEditor({
  group,
  anchor,
  focusName,
}: {
  group: GroupId;
  anchor: Anchor;
  focusName: boolean;
}) {
  const app = useShell();
  const entry = useStore(app.layout, (state) => selectGroup(state, group));
  const members = useStore(app.layout, (state) => selectGroupTabs(state, group).join(" "));
  const total = useStore(app.layout, (state) => selectTabOrder(state).length);
  // Where the group stands among the row's groups and lone tabs.
  const place = useStore(app.layout, (state) => {
    const units = stripItems(state).filter((item) => item.kind === "group" || item.group === null);
    const at = units.findIndex((item) => item.kind === "group" && item.group.id === group);
    return `${at}/${units.length}`;
  });
  const [name, setName] = useState(entry?.name ?? "");
  const [icons, setIcons] = useState(false);
  const [moving, setMoving] = useState(false);
  const [workspaces, setWorkspaces] = useState<WorkspaceList | null>(null);
  const [where, setWhere] = useState(anchor);
  const field = useRef<HTMLInputElement>(null);
  const pending = useRef<number | null>(null);
  const written = useRef(entry?.name ?? "");

  // The editor stays under its label when the label moves.
  useLayoutEffect(() => {
    const label = document.querySelector(`[data-tab-id="${labelId(group)}"]`);
    if (label) setWhere(anchorOf(label));
  }, [group, place, members]);

  useEffect(() => {
    if (focusName) {
      field.current?.focus();
      field.current?.select();
    }
  }, [focusName]);

  useEffect(() => {
    if (!moving || workspaces) return;
    app.client
      .workspacesList()
      .then(setWorkspaces, () => setWorkspaces({ active: "", workspaces: [] }));
  }, [app, moving, workspaces]);

  const run = (command: string, args: Record<string, unknown> = {}) =>
    app.commands.run(command, { group, ...args });

  const writeName = (value: string) => {
    if (pending.current !== null) window.clearTimeout(pending.current);
    pending.current = null;
    const kept = value.trim();
    if (kept === written.current) return;
    written.current = kept;
    run("lume.group.set", { name: kept === "" ? null : kept });
  };

  const close = () => {
    writeName(field.current?.value ?? name);
    app.tabUi.getState().close();
    app.views.focus();
  };

  // A group that is gone has nothing to edit.
  useEffect(() => {
    if (!entry) app.tabUi.getState().close();
  }, [app, entry]);
  useEffect(
    () => () => {
      if (pending.current !== null) window.clearTimeout(pending.current);
    },
    [],
  );
  if (!entry) return null;

  const count = members === "" ? 0 : members.split(" ").length;
  const [at, units] = place.split("/").map(Number) as [number, number];
  const everything = count >= total;
  const others = (workspaces?.workspaces ?? []).filter(
    (workspace) => workspace.name !== workspaces?.active,
  );
  const folded = entry.collapsed === true;

  /** Does something to the group once its label and tabs have gone their way. */
  const then = (exit: "groupExit" | "groupFade", command: string, args = {}) => {
    app.tabUi.getState().close();
    const row = app.tabUi.getState().row;
    const go = () => run(command, args);
    if (row) void row[exit](group).then(go);
    else go();
  };

  return (
    <AnchoredMenu anchor={where} width={GROUP_EDITOR_WIDTH} className="group-editor-place">
      <div className="menu-backdrop" onPointerDown={close} />
      <div
        className="group-editor"
        role="dialog"
        aria-label="Edit tab group"
        data-group-color={entry.color}
        onKeyDown={(event) => {
          if (event.key === "Escape" || (event.key === "Enter" && event.target === field.current)) {
            event.preventDefault();
            close();
          }
          // Keys pressed here are for the editor, not for the terminal behind it.
          event.stopPropagation();
        }}
      >
        <div className="group-editor-top">
          <button
            type="button"
            className="group-editor-picture"
            aria-label="Group picture"
            title="Group picture"
            aria-expanded={icons}
            onClick={() => setIcons(!icons)}
          >
            {entry.icon === undefined ? (
              <span className="group-editor-dot" />
            ) : (
              <GroupGlyph icon={entry.icon} size={15} />
            )}
          </button>
          <input
            ref={field}
            className="group-editor-name"
            aria-label="Group name"
            placeholder="Name this group"
            spellCheck={false}
            autoComplete="off"
            value={name}
            onChange={(event) => {
              const value = event.target.value;
              setName(value);
              if (pending.current !== null) window.clearTimeout(pending.current);
              pending.current = window.setTimeout(() => writeName(value), GROUP_NAME_WRITE_MS);
            }}
          />
        </div>
        <div className="group-editor-icons" data-open={icons}>
          <div role="group" aria-label="Pictures">
            {[null, ...GROUP_ICONS].map((icon: GroupIcon | null) => (
              <button
                key={icon ?? "none"}
                type="button"
                className="control"
                tabIndex={icons ? 0 : -1}
                aria-pressed={(entry.icon ?? null) === icon}
                aria-label={icon === null ? "No picture" : GROUP_ICON_NAMES[icon]}
                title={icon === null ? "No picture" : GROUP_ICON_NAMES[icon]}
                onClick={() => run("lume.group.set", { icon })}
              >
                {icon === null ? (
                  <span className="group-editor-dot" data-small="true" />
                ) : (
                  <GroupGlyph icon={icon} size={14} />
                )}
              </button>
            ))}
          </div>
        </div>
        <div className="group-editor-colors" role="radiogroup" aria-label="Group colour">
          {TAB_COLORS.map((color) => (
            <button
              key={color}
              type="button"
              role="radio"
              className="group-editor-color"
              aria-checked={entry.color === color}
              aria-label={TAB_COLOR_NAMES[color]}
              title={TAB_COLOR_NAMES[color]}
              // One stop for the Tab key: the colour chosen. The arrows choose another.
              tabIndex={entry.color === color ? 0 : -1}
              style={{ background: tabColorValue(color) }}
              onClick={() => run("lume.group.set", { color })}
              onKeyDown={(event) => {
                const colors = event.currentTarget.parentElement;
                if (!colors || !roveWithin(event, colors, ".group-editor-color", "horizontal"))
                  return;
                event.preventDefault();
                const reached = document.activeElement;
                if (reached instanceof HTMLElement) reached.click();
              }}
            />
          ))}
        </div>
        <div className="group-editor-rows" role="menu" aria-label="Group">
          <Row
            glyph="plus"
            label="New tab in group"
            onChoose={() => {
              close();
              app.commands.run("lume.tab.new", { group });
            }}
          />
          <Row
            glyph={folded ? "unfold-horizontal" : "fold-horizontal"}
            label={folded ? "Expand group" : "Collapse group"}
            hint="click label"
            onChoose={() => {
              close();
              run("lume.group.toggle");
            }}
          />
          <Row
            glyph="chevron-left"
            label="Move group left"
            disabled={at <= 0}
            onChoose={() => run("lume.group.move-left")}
          />
          <Row
            glyph="chevron-right"
            label="Move group right"
            disabled={at < 0 || at >= units - 1}
            onChoose={() => run("lume.group.move-right")}
          />
          <Row
            glyph="folder-input"
            label="Move group to workspace"
            disabled={everything}
            expanded={moving}
            onChoose={() => setMoving(!moving)}
          />
          <div className="group-editor-workspaces" data-open={moving}>
            <div>
              {workspaces && others.length === 0 && (
                <p className="menu-note">There is no other workspace.</p>
              )}
              {others.map((workspace) => (
                <button
                  key={workspace.name}
                  type="button"
                  className="menu-item group-editor-row"
                  role="menuitem"
                  tabIndex={moving ? 0 : -1}
                  onClick={() =>
                    then("groupFade", "lume.group.move-to-workspace", {
                      workspace: workspace.name,
                    })
                  }
                >
                  <span className="menu-icon" />
                  <span className="menu-label">{workspace.name}</span>
                </button>
              ))}
            </div>
          </div>
          <Row
            glyph="bookmark"
            label="Keep group"
            hint={entry.kept ? "On" : "Off"}
            pressed={entry.kept === true}
            onChoose={() => run("lume.group.set", { kept: !entry.kept })}
          />
          <div className="menu-separator" role="separator" />
          <Row
            glyph="ungroup"
            label="Ungroup"
            onChoose={() => then("groupExit", "lume.group.ungroup")}
          />
          <Row
            glyph="square-x"
            label="Close group"
            danger
            disabled={everything}
            hint={tabCount(count)}
            onChoose={() => then("groupFade", "lume.group.close")}
          />
        </div>
      </div>
    </AnchoredMenu>
  );
}
