import { Mod } from "@lume/vt";
import { describe, expect, it } from "vitest";

import {
  gridPoint,
  mouseButton,
  pointerModifiers,
  pointerRoute,
  WheelAccumulator,
  wheelRoute,
} from "../src/terminal/mouse.ts";
import { WHEEL_PIXELS_PER_LINE } from "../src/tuning.ts";

describe("who the pointer belongs to", () => {
  it("is the program when it tracks the mouse", () => {
    expect(pointerRoute(true, false)).toBe("report");
  });

  it("is the terminal when the program does not, or when Shift is held", () => {
    expect(pointerRoute(false, false)).toBe("select");
    expect(pointerRoute(false, true)).toBe("select");
    expect(pointerRoute(true, true)).toBe("select");
  });
});

describe("what the wheel does", () => {
  it("is reported to a program that tracks the mouse", () => {
    expect(wheelRoute(true, false, false, false)).toBe("report");
    expect(wheelRoute(true, false, true, true)).toBe("report");
  });

  it("becomes arrow keys for a full-screen program that asked for them", () => {
    expect(wheelRoute(false, false, true, true)).toBe("arrows");
  });

  it("scrolls the terminal's own scrollback otherwise", () => {
    expect(wheelRoute(false, false, false, false)).toBe("scroll");
    // A full-screen program that did not ask for arrow keys gets nothing.
    expect(wheelRoute(false, false, true, false)).toBe("scroll");
    // The mode alone means nothing on the main screen.
    expect(wheelRoute(false, false, false, true)).toBe("scroll");
  });

  it("scrolls the scrollback with Shift whatever the program wants", () => {
    expect(wheelRoute(true, true, true, true)).toBe("scroll");
    expect(wheelRoute(false, true, true, true)).toBe("scroll");
  });
});

describe("turning wheel movement into lines", () => {
  const pixels = (deltaY: number) => ({ deltaY, deltaMode: 0 });

  it("makes three lines of one notch of a mouse wheel", () => {
    const wheel = new WheelAccumulator();
    expect(wheel.lines(pixels(100), 24)).toBe(3);
    expect(wheel.lines(pixels(-100), 24)).toBe(-3);
  });

  it("adds up small movements until they make a line", () => {
    const wheel = new WheelAccumulator();
    const step = WHEEL_PIXELS_PER_LINE / 4;
    expect(wheel.lines(pixels(step), 24)).toBe(0);
    expect(wheel.lines(pixels(step), 24)).toBe(0);
    expect(wheel.lines(pixels(step), 24)).toBe(0);
    expect(wheel.lines(pixels(step), 24)).toBe(1);
    expect(wheel.lines(pixels(step), 24)).toBe(0);
  });

  it("loses nothing over many small movements", () => {
    const wheel = new WheelAccumulator();
    let total = 0;
    for (let i = 0; i < 1000; i++) total += wheel.lines(pixels(7), 24);
    expect(total).toBe(Math.trunc((1000 * 7) / WHEEL_PIXELS_PER_LINE));
  });

  it("starts afresh when the direction changes", () => {
    const wheel = new WheelAccumulator();
    const most = WHEEL_PIXELS_PER_LINE * 0.9;
    expect(wheel.lines(pixels(most), 24)).toBe(0);
    // What was left of the way down is not first undone on the way up.
    expect(wheel.lines(pixels(-most), 24)).toBe(0);
    expect(wheel.lines(pixels(-most), 24)).toBe(-1);
  });

  it("takes line and page movements as they are", () => {
    const wheel = new WheelAccumulator();
    expect(wheel.lines({ deltaY: 2, deltaMode: 1 }, 24)).toBe(2);
    expect(wheel.lines({ deltaY: 1, deltaMode: 2 }, 24)).toBe(23);
    expect(wheel.lines({ deltaY: -1, deltaMode: 2 }, 1)).toBe(-1);
  });

  it("forgets what was left over when reset", () => {
    const wheel = new WheelAccumulator();
    wheel.lines(pixels(WHEEL_PIXELS_PER_LINE * 0.9), 24);
    wheel.reset();
    expect(wheel.lines(pixels(WHEEL_PIXELS_PER_LINE * 0.9), 24)).toBe(0);
  });
});

describe("mouse buttons", () => {
  it("have the names programs are told", () => {
    expect([0, 1, 2, 3, 4].map(mouseButton)).toEqual(["left", "middle", "right", "eight", "nine"]);
    expect(mouseButton(7)).toBeUndefined();
  });

  it("carry the modifiers held with them", () => {
    const none = { shiftKey: false, ctrlKey: false, altKey: false, metaKey: false };
    expect(pointerModifiers(none)).toBe(0);
    expect(pointerModifiers({ ...none, shiftKey: true, altKey: true })).toBe(Mod.Shift | Mod.Alt);
    expect(pointerModifiers({ ...none, ctrlKey: true, metaKey: true })).toBe(Mod.Ctrl | Mod.Super);
  });
});

describe("the cell under the pointer", () => {
  const grid = { cols: 80, rows: 24, cellWidth: 8, cellHeight: 16, padding: 4 };

  it("is found from the padding and the cell size", () => {
    expect(gridPoint(4, 4, grid)).toEqual({ col: 0, row: 0, x: 4, y: 4 });
    expect(gridPoint(11.9, 19.9, grid)).toMatchObject({ col: 0, row: 0 });
    expect(gridPoint(12, 20, grid)).toMatchObject({ col: 1, row: 1 });
    expect(gridPoint(4 + 79 * 8 + 7, 4 + 23 * 16 + 15, grid)).toMatchObject({ col: 79, row: 23 });
  });

  it("is held inside the grid, while the position stays where the pointer is", () => {
    expect(gridPoint(0, -30, grid)).toEqual({ col: 0, row: 0, x: 0, y: -30 });
    expect(gridPoint(5000, 5000, grid)).toMatchObject({ col: 79, row: 23, x: 5000, y: 5000 });
  });
});
