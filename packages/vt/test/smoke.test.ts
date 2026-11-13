import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

import { beforeAll, describe, expect, it } from "vitest";

import { Vt } from "../src/index.ts";

const wasmPath = fileURLToPath(new URL("../dist/ghostty-vt.wasm", import.meta.url));

let vt: Vt;

beforeAll(async () => {
  let bytes: Uint8Array;
  try {
    bytes = await readFile(wasmPath);
  } catch {
    throw new Error(`${wasmPath} is missing. Run \`cargo xtask vt wasm\` first.`);
  }
  vt = await Vt.instantiate(bytes);
});

describe("libghostty-vt (WebAssembly)", () => {
  it("reports the pinned library version", async () => {
    const tools = await readFile(new URL("../../../tools.toml", import.meta.url), "utf8");
    const pinned = /\[ghostty-vt\][^[]*?version = "([^"]+)"/s.exec(tools)?.[1];
    expect(pinned).toBeTruthy();
    expect(vt.version).toBe(pinned);
  });

  // The same smoke test as the Rust `lume-vt` crate, so both builds are held to one result.
  it("writes text and reads back text and colour", () => {
    const term = vt.createTerminal(80, 24);
    try {
      term.write("hello\r\n\x1b[31mred");

      expect(term.rowText(0)).toBe("hello");
      expect(term.rowText(1)).toBe("red");

      const cell = term.cell(0, 1);
      expect(cell.text).toBe("r");
      expect(cell.fg).toEqual({ kind: "palette", index: 1 });
      expect(term.cell(0, 0).fg).toEqual({ kind: "default" });
    } finally {
      term.free();
    }
  });

  it("gives wide characters two columns and one place in the text", () => {
    const term = vt.createTerminal(10, 2);
    try {
      term.write("a中b");
      expect(term.rowText(0)).toBe("a中b");
      expect(term.cell(1, 0).wide).toBe("wide");
      expect(term.cell(2, 0).wide).toBe("spacer-tail");
    } finally {
      term.free();
    }
  });

  it("resizes the grid", () => {
    const term = vt.createTerminal(10, 2);
    try {
      term.resize(20, 5);
      expect([term.cols, term.rows]).toEqual([20, 5]);
      expect(() => term.cell(19, 4)).not.toThrow();
      expect(() => term.cell(20, 0)).toThrow(RangeError);
    } finally {
      term.free();
    }
  });

  it("tracks the cursor and the alternate screen", () => {
    const term = vt.createTerminal(20, 5);
    try {
      term.write("abc");
      expect(term.cursor()).toMatchObject({ x: 3, y: 0, visible: true });
      expect(term.alternateScreen()).toBe(false);
      term.write("\x1b[?1049h\x1b[?25l");
      expect(term.alternateScreen()).toBe(true);
      expect(term.cursor().visible).toBe(false);
    } finally {
      term.free();
    }
  });

  it("survives many terminals and large writes", () => {
    const big = new TextEncoder().encode("x".repeat(200_000) + "\r\nend");
    for (let i = 0; i < 20; i++) {
      const term = vt.createTerminal(120, 40);
      term.write(big);
      expect(term.rowText(term.cursor().y)).toBe("end");
      term.free();
    }
  });
});
