import { beforeEach, describe, expect, it, vi } from "vitest";

type Marks = typeof import("./start-marks.ts");

async function fresh(): Promise<Marks> {
  vi.resetModules();
  return import("./start-marks.ts");
}

describe("the steps of a page's start", () => {
  let marks: Marks;
  beforeEach(async () => {
    marks = await fresh();
  });

  it("keeps the first time each step was reached", () => {
    marks.startStep("vt", 10);
    marks.startStep("vt", 20);
    expect(marks.startSteps().vt).toBe(10);
    expect(marks.startSteps().parsed).toBeTypeOf("number");
  });

  it("ends the start at the first frame drawn after output came", () => {
    marks.startDrawn();
    expect(marks.startSteps().frame).toBeTypeOf("number");
    expect(marks.startSteps().drawn).toBeUndefined();
    marks.startOutput();
    marks.startDrawn();
    expect(marks.startSteps().drawn).toBeTypeOf("number");
    expect(marks.startSteps().output).toBeLessThanOrEqual(marks.startSteps().drawn ?? 0);
  });

  it("says when a terminal has drawn its first frame, output or none", async () => {
    let drawn = false;
    void marks.firstFrame.then(() => {
      drawn = true;
    });
    await Promise.resolve();
    expect(drawn).toBe(false);
    marks.startDrawn();
    await marks.firstFrame;
    expect(drawn).toBe(true);
  });
});
