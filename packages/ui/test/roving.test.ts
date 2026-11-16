import { describe, expect, it } from "vitest";

import { imageType } from "../src/shell/background.ts";
import { rovingTarget } from "../src/shell/roving.ts";

describe("moving the keyboard within a group", () => {
  it("goes to the neighbours along the group's direction, and round at the ends", () => {
    expect(rovingTarget("ArrowRight", 0, 3, "horizontal")).toBe(1);
    expect(rovingTarget("ArrowRight", 2, 3, "horizontal")).toBe(0);
    expect(rovingTarget("ArrowLeft", 1, 3, "horizontal")).toBe(0);
    expect(rovingTarget("ArrowLeft", 0, 3, "horizontal")).toBe(2);
    expect(rovingTarget("ArrowDown", 0, 3, "vertical")).toBe(1);
    expect(rovingTarget("ArrowUp", 0, 3, "vertical")).toBe(2);
  });

  it("goes to the first and the last with Home and End", () => {
    expect(rovingTarget("Home", 2, 4, "horizontal")).toBe(0);
    expect(rovingTarget("End", 0, 4, "vertical")).toBe(3);
  });

  it("is not moved by the arrows across the group's direction, or by other keys", () => {
    expect(rovingTarget("ArrowUp", 1, 3, "horizontal")).toBeUndefined();
    expect(rovingTarget("ArrowLeft", 1, 3, "vertical")).toBeUndefined();
    expect(rovingTarget("Enter", 1, 3, "horizontal")).toBeUndefined();
    expect(rovingTarget("a", 1, 3, "horizontal")).toBeUndefined();
  });

  it("moves nothing in a group that is empty or from a member that is not in it", () => {
    expect(rovingTarget("ArrowRight", 0, 0, "horizontal")).toBeUndefined();
    expect(rovingTarget("ArrowRight", -1, 3, "horizontal")).toBeUndefined();
    expect(rovingTarget("End", 3, 3, "horizontal")).toBeUndefined();
  });

  it("stays put in a group of one", () => {
    expect(rovingTarget("ArrowRight", 0, 1, "horizontal")).toBe(0);
  });
});

describe("telling what kind of picture bytes are", () => {
  const bytes = (...values: number[]) => Uint8Array.from(values);
  const ascii = (text: string) => [...text].map((letter) => letter.charCodeAt(0));

  it("knows the four kinds the page shows by how each starts", () => {
    expect(imageType(bytes(0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0))).toBe(
      "image/png",
    );
    expect(imageType(bytes(0xff, 0xd8, 0xff, 0xe0, 0, 0x10))).toBe("image/jpeg");
    expect(imageType(bytes(...ascii("RIFF"), 1, 2, 3, 4, ...ascii("WEBPVP8 ")))).toBe("image/webp");
    expect(imageType(bytes(...ascii("GIF89a"), 1, 0))).toBe("image/gif");
    expect(imageType(bytes(...ascii("GIF87a"), 1, 0))).toBe("image/gif");
  });

  it("knows nothing else for a picture: not a drawing in text, not another RIFF file", () => {
    expect(imageType(bytes(...ascii("<svg xmlns=")))).toBeUndefined();
    expect(imageType(bytes(...ascii("RIFF"), 1, 2, 3, 4, ...ascii("WAVEfmt ")))).toBeUndefined();
    expect(imageType(bytes(...ascii("GIF88a")))).toBeUndefined();
  });

  it("knows nothing of bytes that end before a kind's beginning does", () => {
    expect(imageType(new Uint8Array(0))).toBeUndefined();
    expect(imageType(bytes(0x89, 0x50, 0x4e))).toBeUndefined();
    expect(imageType(bytes(...ascii("RIFF"), 1, 2, 3, 4, ...ascii("WE")))).toBeUndefined();
  });
});
