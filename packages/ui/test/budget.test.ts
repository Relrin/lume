import { describe, expect, it } from "vitest";

import {
  SERVED_CELLS_FLOOR,
  SERVED_CELLS_MAX,
  SERVED_ENGINE_MAX_BYTES,
  SERVED_SCROLLBACK_FLOOR_BYTES,
  SERVED_SCROLLBACK_MAX_BYTES,
  SERVED_TERMINALS_MAX,
  TERMINAL_CELLS_MAX,
} from "../src/limits.ts";
import { ServedBudget, type ServedShare } from "../src/terminal/budget.ts";
import { withinCells } from "../src/transport/types.ts";

const MIB = 1024 * 1024;
const cells = (size: { cols: number; rows: number }) => size.cols * size.rows;

function budget(engineBytes = 0): ServedBudget {
  return new ServedBudget(() => engineBytes);
}

/** Opens terminals until no more may be. */
function every(from: ServedBudget): ServedShare[] {
  const open: ServedShare[] = [];
  for (let share = from.open(); share; share = from.open()) open.push(share);
  return open;
}

describe("the most cells of a terminal", () => {
  it("leaves a size that has no more as it is", () => {
    expect(withinCells({ cols: 80, rows: 24 })).toEqual({ cols: 80, rows: 24 });
    expect(withinCells({ cols: 4096, rows: 128 })).toEqual({ cols: 4096, rows: 128 });
  });

  it("has the longer side give way", () => {
    expect(withinCells({ cols: 4096, rows: 200 })).toEqual({ cols: 2621, rows: 200 });
    expect(withinCells({ cols: 200, rows: 4096 })).toEqual({ cols: 200, rows: 2621 });
    expect(withinCells({ cols: 4096, rows: 4096 })).toEqual({ cols: 724, rows: 724 });
  });

  it("never makes a side of nothing, nor more than was asked", () => {
    for (const max of [1, 7, 1920, TERMINAL_CELLS_MAX]) {
      for (const cols of [1, 2, 80, 1000, 4096]) {
        for (const rows of [1, 2, 24, 1000, 4096]) {
          const size = withinCells({ cols, rows }, max);
          expect(Math.min(size.cols, size.rows)).toBeGreaterThanOrEqual(1);
          expect(cells(size)).toBeLessThanOrEqual(max);
          expect(size.cols).toBeLessThanOrEqual(cols);
          expect(size.rows).toBeLessThanOrEqual(rows);
        }
      }
    }
  });
});

describe("what servers' terminals may hold of a window", () => {
  it("lets the first few keep as much scrollback as their users would want", () => {
    const of = budget();
    // Forty thousand lines of two hundred columns take about this much.
    expect(of.open()?.scrollbackBytes).toBe(64 * MIB);
    const tenth = Array.from({ length: 9 }, () => of.open()).at(-1);
    expect(tenth?.scrollbackBytes).toBeGreaterThan(32 * MIB);
  });

  it("bounds the scrollback of all of them together", () => {
    const open = every(budget());
    expect(open).toHaveLength(SERVED_TERMINALS_MAX);
    const kept = open.reduce((sum, share) => sum + share.scrollbackBytes, 0);
    expect(kept).toBeLessThan(
      SERVED_SCROLLBACK_MAX_BYTES + SERVED_TERMINALS_MAX * SERVED_SCROLLBACK_FLOOR_BYTES,
    );
    for (const share of open) {
      expect(share.scrollbackBytes).toBeGreaterThanOrEqual(SERVED_SCROLLBACK_FLOOR_BYTES);
    }
  });

  it("gives back what a terminal that is gone held", () => {
    const of = budget();
    const open = every(of);
    expect(of.open()).toBeUndefined();
    for (const share of open) {
      share.close();
      share.close();
    }
    expect(of.open()?.scrollbackBytes).toBe(64 * MIB);
    expect(every(of)).toHaveLength(SERVED_TERMINALS_MAX - 1);
  });

  it("makes none while the engine holds too much as it is", () => {
    expect(budget(SERVED_ENGINE_MAX_BYTES).open()).toBeDefined();
    expect(budget(SERVED_ENGINE_MAX_BYTES + 1).open()).toBeUndefined();
  });

  it("leaves terminals of an ordinary size as they are, however many there are", () => {
    for (const share of every(budget()).slice(0, 100)) {
      expect(share.fit({ cols: 200, rows: 50 })).toEqual({ cols: 200, rows: 50 });
    }
  });

  it("bounds the cells of all of them together", () => {
    const held = every(budget()).map((share) => cells(share.fit({ cols: 4096, rows: 4096 })));
    expect(Math.max(...held)).toBeLessThanOrEqual(TERMINAL_CELLS_MAX);
    expect(Math.min(...held)).toBeGreaterThan(SERVED_CELLS_FLOOR / 2);
    const sum = held.reduce((total, one) => total + one, 0);
    expect(sum).toBeLessThanOrEqual(SERVED_CELLS_MAX + SERVED_TERMINALS_MAX * SERVED_CELLS_FLOOR);
  });

  it("counts a terminal by the size it has now", () => {
    const of = budget();
    const open = Array.from({ length: 9 }, () => of.open()!);
    for (const share of open) share.fit({ cols: 4096, rows: 4096 });
    // Asked again, one that is there has what it had: it does not stand in its own way.
    const fourth = open[3]!;
    expect(cells(fourth.fit({ cols: 4096, rows: 4096 }))).toBeGreaterThan(500_000);
    const last = open.at(-1)!;
    expect(cells(last.fit({ cols: 1000, rows: 500 }))).toBeLessThan(500_000);
    // Made small again, the others leave room.
    for (const share of open.slice(0, -1)) share.fit({ cols: 80, rows: 24 });
    expect(last.fit({ cols: 1000, rows: 500 })).toEqual({ cols: 1000, rows: 500 });
    // One that is gone holds nothing.
    for (const share of open) share.close();
    expect(of.open()!.fit({ cols: 1000, rows: 500 })).toEqual({ cols: 1000, rows: 500 });
  });
});
