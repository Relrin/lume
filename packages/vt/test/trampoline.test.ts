import { describe, expect, it } from "vitest";

import { thunkModule, Trampoline } from "../src/trampoline.ts";
import { newCore } from "./support.ts";

function emptyTable(): WebAssembly.Table {
  return new WebAssembly.Table({ element: "anyfunc", initial: 0 });
}

describe("thunk modules", () => {
  it("is a valid module for every value type", () => {
    const bytes = thunkModule({ params: ["i32", "i64", "f32", "f64"], results: ["i32"] });
    expect(WebAssembly.validate(bytes)).toBe(true);
  });

  it("encodes parameter counts above 127 in more than one byte", () => {
    const params = Array.from({ length: 200 }, () => "i32" as const);
    expect(WebAssembly.validate(thunkModule({ params, results: [] }))).toBe(true);
  });
});

describe("the trampoline", () => {
  it("makes a JavaScript function callable through the table", () => {
    const table = emptyTable();
    const trampoline = new Trampoline(table);
    const index = trampoline.install(
      { params: ["i32", "i32"], results: ["i32"] },
      (a: number, b: number) => a + b,
    );
    expect(trampoline.get(index)(2, 40)).toBe(42);
  });

  it("passes 64-bit and floating-point values through unchanged", () => {
    const trampoline = new Trampoline(emptyTable());
    const seen: unknown[] = [];
    const index = trampoline.install(
      { params: ["i64", "f32", "f64"], results: ["f64"] },
      (a: bigint, b: number, c: number) => {
        seen.push(a, b, c);
        return c * 2;
      },
    );
    expect(trampoline.get(index)(2n ** 40n, 1.5, 0.25)).toBe(0.5);
    expect(seen).toEqual([2n ** 40n, 1.5, 0.25]);
  });

  it("gives each function its own slot and reuses a released one", () => {
    const table = emptyTable();
    const trampoline = new Trampoline(table);
    const signature = { params: [], results: ["i32"] } as const;
    const one = trampoline.install(signature, () => 1);
    const two = trampoline.install(signature, () => 2);
    expect(one).not.toBe(two);
    expect(table.length).toBe(2);

    trampoline.release(one);
    expect(() => trampoline.get(one)).toThrow(/empty/);
    const three = trampoline.install(signature, () => 3);
    expect(three).toBe(one);
    expect(table.length).toBe(2);
    expect(trampoline.get(three)()).toBe(3);
    expect(trampoline.get(two)()).toBe(2);
  });

  it("rejects a call with the wrong signature instead of running the function", async () => {
    // The module checks the signature of an indirect call, so a thunk built for the
    // wrong one traps rather than passing misplaced arguments.
    const core = await newCore();
    let called = false;
    const wrong = core.trampoline.install({ params: ["i32"], results: [] }, () => {
      called = true;
    });
    const handle = core.newHandle("ghostty_terminal_new", (out) =>
      core.x.ghostty_terminal_new(0, out, 80, 24),
    );
    const bell = core.manifest.enumValue("GhosttyTerminalOption", "BELL");
    core.check(core.x.ghostty_terminal_set(handle, bell, wrong), "ghostty_terminal_set");
    expect(() => core.x.ghostty_terminal_vt_write(handle, core.stage(Uint8Array.of(7)), 1)).toThrow(
      WebAssembly.RuntimeError,
    );
    expect(called).toBe(false);
  });

  it("returns a struct through the hidden result pointer the manifest describes", async () => {
    const core = await newCore();
    const { manifest, trampoline, x } = core;
    const option = (name: string) => manifest.enumValue("GhosttyTerminalOption", name);

    const enquiry = manifest.callback("GhosttyTerminalEnquiryFn");
    expect(enquiry).toEqual({ params: ["i32", "i32", "i32"], results: [], resultPointer: true });

    const answer = core.alloc(4);
    core.u8().set(new TextEncoder().encode("lume"), answer);
    const replies: string[] = [];
    const handle = core.newHandle("ghostty_terminal_new", (out) =>
      x.ghostty_terminal_new(0, out, 80, 24),
    );
    core.check(
      x.ghostty_terminal_set(
        handle,
        option("WRITE_PTY"),
        trampoline.install(
          manifest.callback("GhosttyTerminalWritePtyFn"),
          (_terminal: number, _userdata: number, data: number, len: number) => {
            replies.push(core.string(data, len));
          },
        ),
      ),
      "ghostty_terminal_set",
    );
    core.check(
      x.ghostty_terminal_set(
        handle,
        option("ENQUIRY"),
        trampoline.install(enquiry, (result: number) => {
          core.view().setUint32(result + manifest.offset("GhosttyString", "ptr"), answer, true);
          core.view().setUint32(result + manifest.offset("GhosttyString", "len"), 4, true);
        }),
      ),
      "ghostty_terminal_set",
    );

    x.ghostty_terminal_vt_write(handle, core.stage(Uint8Array.of(5)), 1);
    expect(replies).toEqual(["lume"]);
    x.ghostty_terminal_free(handle);
  });
});
