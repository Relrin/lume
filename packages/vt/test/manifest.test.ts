import { describe, expect, it } from "vitest";

import { bitReader, Manifest } from "../src/manifest.ts";
import { loadVt } from "./support.ts";

const abi = { pointer_size: 4, endian: "little" };

describe("the type manifest", () => {
  it("rejects a schema it does not know", () => {
    expect(() => new Manifest(JSON.stringify({ schema: 2, abi, types: {} }))).toThrow(/schema 2/);
  });

  it("rejects a module built for another pointer size", () => {
    const json = JSON.stringify({
      schema: 1,
      abi: { pointer_size: 8, endian: "little" },
      types: {},
    });
    expect(() => new Manifest(json)).toThrow(/32-bit/);
  });

  it("names the descriptor that is missing", () => {
    const manifest = new Manifest(JSON.stringify({ schema: 1, abi, types: {} }));
    expect(() => manifest.struct("GhosttyPoint")).toThrow(/GhosttyPoint/);
    expect(() => manifest.enumValue("GhosttyCellWide", "WIDE")).toThrow(/GhosttyCellWide/);
  });

  it("names a field or value the descriptor lacks", async () => {
    const { manifest } = (await loadVt()).core;
    expect(() => manifest.offset("GhosttyPoint", "nope")).toThrow(/no field nope/);
    expect(() => manifest.enumValue("GhosttyCellWide", "NOPE")).toThrow(/no value NOPE/);
  });

  it("describes the layouts and values the wrapper reads", async () => {
    const { manifest } = (await loadVt()).core;
    expect(manifest.struct("GhosttyGridRef").size).toBe(8);
    expect(manifest.struct("GhosttyStyle").size).toBe(16);
    expect(manifest.enumValue("GhosttyTerminalOption", "WRITE_PTY")).toBe(2);
    expect(manifest.enumValue("GhosttyCellWide", "SPACER_TAIL")).toBe(2);
    expect(manifest.bits("GhosttyStyleFlags", "underline")).toEqual({ lsb: 8, width: 3 });
    expect(manifest.armBits("GhosttyCell", "content", "CODEPOINT", "codepoint").width).toBe(21);
  });

  it("lowers callback types to WebAssembly signatures", async () => {
    const { manifest } = (await loadVt()).core;
    expect(manifest.callback("GhosttyTerminalWritePtyFn")).toEqual({
      params: ["i32", "i32", "i32", "i32"],
      results: [],
      resultPointer: false,
    });
    expect(manifest.callback("GhosttyTerminalSizeFn")).toEqual({
      params: ["i32", "i32", "i32"],
      results: ["i32"],
      resultPointer: false,
    });
    expect(manifest.callback("GhosttyTerminalXtversionFn").resultPointer).toBe(true);
  });

  it("refuses to guess how a struct parameter is passed", async () => {
    const { manifest } = (await loadVt()).core;
    expect(() => manifest.callback("GhosttyMimeReaderFn")).toThrow(/passed by value/);
  });
});

describe("bit readers", () => {
  // The value 0x_89ab_cdef_0123_4567 as its two halves.
  const lo = 0x01234567;
  const hi = 0x89abcdef;
  const whole = (BigInt(hi) << 32n) | BigInt(lo);
  const expected = (lsb: number, width: number) =>
    Number((whole >> BigInt(lsb)) & ((1n << BigInt(width)) - 1n));

  it.each([
    [0, 2],
    [2, 24],
    [26, 6],
    [26, 16],
    [31, 2],
    [32, 8],
    [42, 2],
    [45, 1],
    [40, 24],
  ])("reads the field at bit %i of width %i", (lsb, width) => {
    expect(bitReader({ lsb, width })(lo, hi)).toBe(expected(lsb, width));
  });

  it("refuses a field that does not fit", () => {
    expect(() => bitReader({ lsb: 60, width: 8 })).toThrow(RangeError);
    expect(() => bitReader({ lsb: 0, width: 32 })).toThrow(RangeError);
  });
});
