import { describe, expect, it } from "vitest";

import { SyncHold } from "../src/terminal/sync-hold.ts";
import { SYNC_HOLD_MAX_MS } from "../src/tuning.ts";

describe("a screen a program is still drawing", () => {
  it("is not shown until the program says it is done", () => {
    const hold = new SyncHold();
    const began: number[] = [];
    const wait = (ms: number) => began.push(ms);
    expect(hold.held(false, 1000, wait)).toBe(false);
    expect(hold.held(true, 1010, wait)).toBe(true);
    expect(hold.held(true, 1020, wait)).toBe(true);
    // One wait is begun for one screen, however often it is asked about.
    expect(began).toEqual([SYNC_HOLD_MAX_MS]);
    expect(hold.held(false, 1030, wait)).toBe(false);
    // The next screen is waited for anew.
    expect(hold.held(true, 1040, wait)).toBe(true);
    expect(began).toHaveLength(2);
  });

  it("is shown as it stands when the program never says so", () => {
    const hold = new SyncHold();
    expect(hold.held(true, 0)).toBe(true);
    expect(hold.held(true, SYNC_HOLD_MAX_MS - 1)).toBe(true);
    expect(hold.held(true, SYNC_HOLD_MAX_MS)).toBe(false);
    // Nor is it waited for again while it goes on drawing the same screen.
    expect(hold.held(true, SYNC_HOLD_MAX_MS * 10)).toBe(false);
    expect(hold.held(false, SYNC_HOLD_MAX_MS * 10 + 1)).toBe(false);
    expect(hold.held(true, SYNC_HOLD_MAX_MS * 10 + 2)).toBe(true);
  });
});
