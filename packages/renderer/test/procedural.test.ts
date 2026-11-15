import { describe, expect, it } from "vitest";

import { UnderlineStyle } from "../src/frame.ts";
import { cellMetrics } from "../src/metrics.ts";
import { boxShapes, isProcedural } from "../src/procedural/box.ts";
import { rasterise, type Mask } from "../src/procedural/coverage.ts";
import { underlineSprite } from "../src/procedural/underline.ts";

const W = 10;
const H = 20;
const T = 2;

function draw(codepoint: number, width = W, height = H, thickness = T): Mask {
  const shapes = boxShapes(codepoint, width, height, thickness);
  if (!shapes) throw new Error(`U+${codepoint.toString(16)} has no shapes`);
  return rasterise(shapes, width, height);
}

const at = (mask: Mask, x: number, y: number) => mask.pixels[y * mask.width + x]!;

/** The mask as rows of characters: `#` covered, `+` partly covered, `.` empty. */
function picture(mask: Mask): string[] {
  const rows: string[] = [];
  for (let y = 0; y < mask.height; y++) {
    let row = "";
    for (let x = 0; x < mask.width; x++) {
      const value = at(mask, x, y);
      row += value === 255 ? "#" : value === 0 ? "." : "+";
    }
    rows.push(row);
  }
  return rows;
}

/** Whether a mask has ink on each edge: top, right, bottom, left. */
function edges(mask: Mask): string {
  const any = (points: [number, number][]) => points.some(([x, y]) => at(mask, x, y) > 0);
  const range = (n: number) => Array.from({ length: n }, (_, i) => i);
  return [
    any(range(mask.width).map((x) => [x, 0])),
    any(range(mask.height).map((y) => [mask.width - 1, y])),
    any(range(mask.width).map((x) => [x, mask.height - 1])),
    any(range(mask.height).map((y) => [0, y])),
  ]
    .map((edge) => (edge ? "1" : "0"))
    .join("");
}

describe("which characters are drawn from shapes", () => {
  it("covers box drawing and block elements and nothing else", () => {
    expect(isProcedural(0x24ff)).toBe(false);
    expect(isProcedural(0x2500)).toBe(true);
    expect(isProcedural(0x259f)).toBe(true);
    expect(isProcedural(0x25a0)).toBe(false);
    expect(boxShapes(0x41, W, H, T)).toBeUndefined();
  });

  it("has shapes with ink for every codepoint in the range", () => {
    for (let codepoint = 0x2500; codepoint <= 0x259f; codepoint++) {
      const mask = draw(codepoint);
      expect(
        mask.pixels.some((value) => value > 0),
        `U+${codepoint.toString(16)}`,
      ).toBe(true);
    }
  });
});

describe("box-drawing lines", () => {
  it("draws a light horizontal line across the middle of the cell", () => {
    expect(picture(draw(0x2500, 6, 6, 2))).toEqual([
      "......",
      "......",
      "######",
      "######",
      "......",
      "......",
    ]);
  });

  it("draws a heavy line twice as thick, centred on the same place", () => {
    expect(picture(draw(0x2501, 6, 6, 1))).toEqual([
      "......",
      "......",
      "######",
      "######",
      "......",
      "......",
    ]);
  });

  it("reaches exactly the edges its arms point to", () => {
    // Up, right, down, left for a sample of every kind of junction.
    const expected: [number, string][] = [
      [0x2500, "0101"],
      [0x2502, "1010"],
      [0x250c, "0110"],
      [0x2510, "0011"],
      [0x2514, "1100"],
      [0x2518, "1001"],
      [0x251c, "1110"],
      [0x2524, "1011"],
      [0x252c, "0111"],
      [0x2534, "1101"],
      [0x253c, "1111"],
      [0x2554, "0110"],
      [0x2557, "0011"],
      [0x255a, "1100"],
      [0x255d, "1001"],
      [0x256c, "1111"],
      [0x2574, "0001"],
      [0x2575, "1000"],
      [0x2576, "0100"],
      [0x2577, "0010"],
      [0x256d, "0110"],
      [0x256e, "0011"],
      [0x256f, "1001"],
      [0x2570, "1100"],
    ];
    for (const [codepoint, arms] of expected) {
      expect(edges(draw(codepoint)), `U+${codepoint.toString(16)}`).toBe(arms);
    }
  });

  it("makes a cross whose lines meet with no gap", () => {
    const mask = draw(0x253c, 6, 6, 2);
    expect(picture(mask)).toEqual(["..##..", "..##..", "######", "######", "..##..", "..##.."]);
  });

  it("makes a square corner with nothing outside it", () => {
    // Down and right: the corner pixel is filled and the top-left stays empty.
    expect(picture(draw(0x250c, 6, 6, 2))).toEqual([
      "......",
      "......",
      "..####",
      "..####",
      "..##..",
      "..##..",
    ]);
  });

  it("turns a double corner with one line inside the other", () => {
    expect(picture(draw(0x2554, 9, 9, 1))).toEqual([
      ".........",
      ".........",
      ".........",
      "...######",
      "...#.....",
      "...#.####",
      "...#.#...",
      "...#.#...",
      "...#.#...",
    ]);
  });

  it("leaves the middle of a double cross open", () => {
    expect(picture(draw(0x256c, 9, 9, 1))).toEqual([
      "...#.#...",
      "...#.#...",
      "...#.#...",
      "####.####",
      ".........",
      "####.####",
      "...#.#...",
      "...#.#...",
      "...#.#...",
    ]);
  });

  it("stops a single line at the nearer line of a double one it meets", () => {
    // A double vertical line with a single line going right.
    expect(picture(draw(0x255f, 9, 9, 1))).toEqual([
      "...#.#...",
      "...#.#...",
      "...#.#...",
      "...#.#...",
      "...#.####",
      "...#.#...",
      "...#.#...",
      "...#.#...",
      "...#.#...",
    ]);
  });

  it("carries a single line through a double one it crosses", () => {
    expect(picture(draw(0x256a, 9, 9, 1))[3]).toBe("#########");
    expect(picture(draw(0x256a, 9, 9, 1))[4]).toBe("....#....");
    expect(picture(draw(0x256a, 9, 9, 1))[0]).toBe("....#....");
  });

  it("joins the same way as its neighbour on every shared edge", () => {
    // Where a line leaves one cell it must enter the next at the same pixels.
    const column = (mask: Mask, x: number) =>
      Array.from({ length: mask.height }, (_, y) => at(mask, x, y) > 0);
    const row = (mask: Mask, y: number) =>
      Array.from({ length: mask.width }, (_, x) => at(mask, x, y) > 0);
    const horizontal = [0x2500, 0x250c, 0x2510, 0x253c, 0x252c, 0x256d, 0x256e, 0x2574, 0x2576];
    const vertical = [0x2502, 0x250c, 0x2514, 0x253c, 0x251c, 0x256d, 0x2570, 0x2575, 0x2577];
    const line = column(draw(0x2500), 0);
    for (const codepoint of horizontal) {
      const mask = draw(codepoint);
      for (const x of [0, W - 1]) {
        if (column(mask, x).some(Boolean)) {
          expect(column(mask, x), `U+${codepoint.toString(16)} at x=${x}`).toEqual(line);
        }
      }
    }
    const bar = row(draw(0x2502), 0);
    for (const codepoint of vertical) {
      const mask = draw(codepoint);
      for (const y of [0, H - 1]) {
        if (row(mask, y).some(Boolean)) {
          expect(row(mask, y), `U+${codepoint.toString(16)} at y=${y}`).toEqual(bar);
        }
      }
    }
  });

  it("spaces dashes so that they continue evenly into the next cell", () => {
    // Three dashes in twelve pixels: each period is a one-pixel gap and a three-pixel
    // dash, so the row reads the same when cells are put side by side.
    const row = picture(draw(0x2504, 12, 6, 2))[2]!;
    expect(row).toBe(".###.###.###");
    expect((row + row).match(/#+/g)!.every((dash) => dash.length === 3)).toBe(true);
    expect((row + row).match(/\.+/g)!.every((gap) => gap.length === 1)).toBe(true);
  });

  it("antialiases diagonals and fills the corners they run into", () => {
    const mask = draw(0x2572, 10, 10, 2);
    expect(at(mask, 0, 0)).toBeGreaterThan(0);
    expect(at(mask, 9, 9)).toBeGreaterThan(0);
    expect(at(mask, 9, 0)).toBe(0);
    expect(mask.pixels.some((value) => value > 0 && value < 255)).toBe(true);
  });
});

describe("block elements", () => {
  it("fills the whole cell for a full block", () => {
    expect(draw(0x2588).pixels.every((value) => value === 255)).toBe(true);
  });

  it("fills exactly half for the half blocks", () => {
    const count = (mask: Mask) => mask.pixels.filter((value) => value === 255).length;
    expect(count(draw(0x2580))).toBe((W * H) / 2);
    expect(count(draw(0x2584))).toBe((W * H) / 2);
    expect(count(draw(0x258c))).toBe((W * H) / 2);
    expect(count(draw(0x2590))).toBe((W * H) / 2);
    expect(at(draw(0x2580), 0, 0)).toBe(255);
    expect(at(draw(0x2584), 0, 0)).toBe(0);
    expect(at(draw(0x2590), 0, 0)).toBe(0);
  });

  it("makes upper and lower halves tile with no gap or overlap", () => {
    const upper = draw(0x2580, 7, 9);
    const lower = draw(0x2584, 7, 9);
    for (let i = 0; i < upper.pixels.length; i++) {
      expect(upper.pixels[i]! + lower.pixels[i]!).toBe(255);
    }
  });

  it("grows the eighths steadily", () => {
    const filled = (codepoint: number) => draw(codepoint, 8, 16).pixels.filter(Boolean).length;
    const lower = Array.from({ length: 8 }, (_, i) => filled(0x2581 + i));
    expect(lower).toEqual([16, 32, 48, 64, 80, 96, 112, 128]);
    const left = Array.from({ length: 7 }, (_, i) => filled(0x2589 + i));
    expect(left).toEqual([112, 96, 80, 64, 48, 32, 16]);
  });

  it("draws shades as even partial coverage", () => {
    expect(new Set(draw(0x2591).pixels)).toEqual(new Set([64]));
    expect(new Set(draw(0x2592).pixels)).toEqual(new Set([128]));
    expect(new Set(draw(0x2593).pixels)).toEqual(new Set([191]));
  });

  it("fills the named quadrants", () => {
    const quadrants = (codepoint: number) => {
      const mask = draw(codepoint, 4, 4);
      return [at(mask, 0, 0), at(mask, 3, 0), at(mask, 0, 3), at(mask, 3, 3)]
        .map((value) => (value ? "1" : "0"))
        .join("");
    };
    expect(quadrants(0x2596)).toBe("0010");
    expect(quadrants(0x2597)).toBe("0001");
    expect(quadrants(0x2598)).toBe("1000");
    expect(quadrants(0x2599)).toBe("1011");
    expect(quadrants(0x259a)).toBe("1001");
    expect(quadrants(0x259b)).toBe("1110");
    expect(quadrants(0x259c)).toBe("1101");
    expect(quadrants(0x259d)).toBe("0100");
    expect(quadrants(0x259e)).toBe("0110");
    expect(quadrants(0x259f)).toBe("0111");
  });
});

describe("underline sprites", () => {
  const metrics = cellMetrics(
    { advance: 10, ascent: 14, descent: 4 },
    { family: "x", sizePx: 16 },
    0,
    1,
  );
  const sprite = (style: number) => {
    const made = underlineSprite(style, metrics);
    if (!made) throw new Error("no sprite");
    return { ...made, mask: rasterise(made.shapes, metrics.cellWidth, made.height) };
  };

  it("has none for no underline", () => {
    expect(underlineSprite(UnderlineStyle.None, metrics)).toBeUndefined();
  });

  it("stays inside the cell in every style", () => {
    for (const style of [1, 2, 3, 4, 5]) {
      const { top, height } = sprite(style);
      expect(top, `style ${style}`).toBeGreaterThanOrEqual(0);
      expect(top + height, `style ${style}`).toBeLessThanOrEqual(metrics.cellHeight);
    }
  });

  it("draws a single underline as a solid bar", () => {
    const { mask, height } = sprite(UnderlineStyle.Single);
    expect(height).toBe(metrics.lineThickness);
    expect(mask.pixels.every((value) => value === 255)).toBe(true);
  });

  it("draws a double underline as two bars with a gap", () => {
    const { mask } = sprite(UnderlineStyle.Double);
    const t = metrics.lineThickness;
    expect(picture(mask).map((row) => row[0])).toEqual([
      ..."#".repeat(t),
      ...".".repeat(t),
      ..."#".repeat(t),
    ]);
  });

  it("draws dots that repeat a whole number of times across the cell", () => {
    const row = picture(sprite(UnderlineStyle.Dotted).mask)[0]!;
    const dots = row.match(/#+/g) ?? [];
    expect(dots.length).toBeGreaterThan(1);
    // The pattern starts with a dot and ends with a gap, so the next cell continues it.
    expect(row.startsWith("#")).toBe(true);
    expect(row.endsWith(".")).toBe(true);
    expect(new Set(dots.map((dot) => dot.length)).size).toBe(1);
  });

  it("draws one dash per cell with a gap after it", () => {
    const row = picture(sprite(UnderlineStyle.Dashed).mask)[0]!;
    expect(row).toBe("#".repeat(5) + ".".repeat(5));
  });

  it("draws a wave that rises and falls and is unbroken across the cell", () => {
    const { mask } = sprite(UnderlineStyle.Curly);
    const inked = (x: number) =>
      Array.from({ length: mask.height }, (_, y) => at(mask, x, y)).some((value) => value > 0);
    expect(Array.from({ length: mask.width }, (_, x) => inked(x)).every(Boolean)).toBe(true);
    const top = Array.from({ length: mask.width }, (_, x) => at(mask, x, 0) > 0);
    const bottom = Array.from({ length: mask.width }, (_, x) => at(mask, x, mask.height - 1) > 0);
    expect(top.some(Boolean)).toBe(true);
    expect(bottom.some(Boolean)).toBe(true);
    expect(top.every(Boolean)).toBe(false);
    expect(bottom.every(Boolean)).toBe(false);
  });
});
