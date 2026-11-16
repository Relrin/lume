/**
 * Draws status items: one small element for each, the same way for every item of a
 * kind.
 *
 * Everything an item carries is rendered as text or as numbers in a drawing. A bar's
 * length and a line's shape are attributes of the drawing, so nothing here needs a
 * style of its own.
 */

import type { ReactNode } from "react";
import { useStore } from "zustand";

import type { PaneId } from "../layout/types.ts";
import {
  AlertIcon,
  BranchIcon,
  DotIcon,
  FolderIcon,
  PulseIcon,
  TerminalIcon,
} from "../shell/icons.tsx";
import type { StatusStore } from "./registry.ts";
import type { MeterItem, SparklineItem, StatusIconName, StatusItem, StatusSlot } from "./types.ts";

/** The size of a meter's bar, in the units it is drawn in. */
const METER = { width: 36, height: 4 } as const;

/** The size of a sparkline, in the units it is drawn in. */
const SPARKLINE = { width: 48, height: 14, inset: 1.5 } as const;

function icon(name: StatusIconName): ReactNode {
  switch (name) {
    case "terminal":
      return <TerminalIcon />;
    case "branch":
      return <BranchIcon />;
    case "pulse":
      return <PulseIcon />;
    case "folder":
      return <FolderIcon />;
    case "dot":
      return <DotIcon />;
    case "alert":
      return <AlertIcon />;
  }
}

/** A share between nought and one, whatever was given. */
function share(value: number): number {
  return Number.isFinite(value) ? Math.min(Math.max(value, 0), 1) : 0;
}

function Meter({ item }: { item: MeterItem }) {
  const filled = share(item.value);
  return (
    <>
      <span className="status-label">{item.label}</span>
      <svg
        className="status-meter"
        width={METER.width}
        height={METER.height}
        viewBox={`0 0 ${METER.width} ${METER.height}`}
        role="meter"
        aria-label={item.label}
        aria-valuemin={0}
        aria-valuemax={1}
        aria-valuenow={filled}
        aria-valuetext={item.text}
      >
        <rect
          className="status-meter-track"
          width={METER.width}
          height={METER.height}
          rx={METER.height / 2}
        />
        <rect
          className="status-meter-fill"
          width={METER.width * filled}
          height={METER.height}
          rx={METER.height / 2}
        />
      </svg>
      {item.text !== undefined && <span className="status-value">{item.text}</span>}
    </>
  );
}

/** The points of a sparkline, spread over its width and between its own extremes. */
export function sparklinePoints(values: readonly number[]): string {
  const points = values.filter((value) => Number.isFinite(value));
  if (points.length === 0) return "";
  const low = Math.min(...points);
  const high = Math.max(...points);
  const { width, height, inset } = SPARKLINE;
  const step = points.length > 1 ? (width - 2 * inset) / (points.length - 1) : 0;
  return points
    .map((value, index) => {
      // Values that are all alike run along the middle.
      const up = high === low ? 0.5 : (value - low) / (high - low);
      const x = inset + step * index;
      const y = height - inset - up * (height - 2 * inset);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

function Sparkline({ item }: { item: SparklineItem }) {
  return (
    <>
      <span className="status-label">{item.label}</span>
      <svg
        className="status-sparkline"
        width={SPARKLINE.width}
        height={SPARKLINE.height}
        viewBox={`0 0 ${SPARKLINE.width} ${SPARKLINE.height}`}
        role="img"
        aria-label={item.text === undefined ? item.label : `${item.label}: ${item.text}`}
      >
        <polyline
          points={sparklinePoints(item.points)}
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
      {item.text !== undefined && <span className="status-value">{item.text}</span>}
    </>
  );
}

function body(item: StatusItem): ReactNode {
  switch (item.kind) {
    case "text":
      return <span className={item.mono ? "status-value" : "status-text"}>{item.text}</span>;
    case "icon":
      return (
        <>
          <span className="status-icon" role="img" aria-label={item.label}>
            {icon(item.icon)}
          </span>
          {item.show_label && <span className="status-text">{item.label}</span>}
        </>
      );
    case "meter":
      return <Meter item={item} />;
    case "sparkline":
      return <Sparkline item={item} />;
    case "badge":
      return <span className="status-badge">{item.text}</span>;
  }
}

/** One status item. */
export function StatusItemView({ item }: { item: StatusItem }) {
  return (
    <span
      className="status-item"
      data-status={item.id}
      data-kind={item.kind}
      data-tone={item.tone ?? "neutral"}
      title={item.tooltip}
    >
      {body(item)}
    </span>
  );
}

/**
 * Some status items of one slot, in the order given. With `pane`, only the items that
 * name that pane: a pane's header shows its own.
 */
export function StatusList({
  items,
  slot,
  pane,
}: {
  items: readonly StatusItem[];
  slot: StatusSlot;
  pane?: PaneId;
}) {
  const shown = pane === undefined ? items : items.filter((item) => item.pane === pane);
  if (shown.length === 0) return null;
  return (
    <span className="status-slot" data-slot={slot}>
      {shown.map((item) => (
        <StatusItemView key={item.id} item={item} />
      ))}
    </span>
  );
}

/** The status items a registry holds for one slot, kept up to date. */
export function StatusSlotItems({
  store,
  slot,
  pane,
}: {
  store: StatusStore;
  slot: StatusSlot;
  pane?: PaneId;
}) {
  const items = useStore(store, (state) => state.bySlot[slot]);
  return <StatusList items={items} slot={slot} pane={pane} />;
}
