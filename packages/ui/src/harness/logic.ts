/**
 * The core's layout logic, built for WebAssembly from `crates/lume-logic-wasm` by
 * `cargo xtask logic`.
 *
 * The harness's pretend core changes its layout through this instead of by hand, so an
 * intent does in a scene exactly what it does in the application: a pane renamed stays
 * renamed, a tab muted is muted, and an intent the core refuses is refused.
 *
 * Every call is a name and a JSON document in, a JSON document out, through three
 * functions the module exports; nothing else crosses.
 */

import type { Created, Intent, LayoutChange, Workspace } from "../layout/types.ts";

/** What the module exports. */
interface Exports {
  readonly memory: WebAssembly.Memory;
  lume_alloc(len: number): number;
  lume_free(ptr: number, len: number): void;
  lume_call(namePtr: number, nameLen: number, inputPtr: number, inputLen: number): bigint;
}

/** The session started for the pane an intent makes. */
export interface GivenSession {
  readonly backend: string;
  readonly session: string;
  readonly cwd?: string | null;
}

/** A workspace after an intent, what changed, and what the intent made. */
export interface Applied {
  readonly workspace: Workspace;
  readonly changes: readonly LayoutChange[];
  readonly created: Created;
}

/** An intent the core refuses, with the code it refuses it with. */
export class Refused extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

/** The core's layout logic, loaded. */
export class LayoutLogic {
  readonly #exports: Exports;

  private constructor(exports: Exports) {
    this.#exports = exports;
  }

  /** Loads the module from its bytes. */
  static async load(bytes: BufferSource): Promise<LayoutLogic> {
    const { instance } = await WebAssembly.instantiate(bytes, {});
    return new LayoutLogic(instance.exports as unknown as Exports);
  }

  /** Whether an intent needs a session started before it is applied. */
  needs(workspace: Workspace, intent: Intent): boolean {
    return this.#call("layout.needs", { workspace, intent }) !== null;
  }

  /** Applies an intent, as the core does, or throws {@link Refused}. */
  apply(workspace: Workspace, intent: Intent, started?: GivenSession): Applied {
    return this.#call("layout.apply", {
      workspace,
      intent,
      ...(started ? { started } : {}),
    }) as Applied;
  }

  #call(name: string, input: unknown): unknown {
    const exports = this.#exports;
    const encoder = new TextEncoder();
    const nameBytes = encoder.encode(name);
    const inputBytes = encoder.encode(JSON.stringify(input));
    const namePtr = exports.lume_alloc(nameBytes.length);
    const inputPtr = exports.lume_alloc(inputBytes.length);
    new Uint8Array(exports.memory.buffer, namePtr, nameBytes.length).set(nameBytes);
    new Uint8Array(exports.memory.buffer, inputPtr, inputBytes.length).set(inputBytes);
    const packed = exports.lume_call(namePtr, nameBytes.length, inputPtr, inputBytes.length);
    exports.lume_free(namePtr, nameBytes.length);
    exports.lume_free(inputPtr, inputBytes.length);
    const ptr = Number(packed >> 32n);
    const len = Number(packed & 0xffff_ffffn);
    // The memory may have grown during the call: read through a fresh view.
    const text = new TextDecoder().decode(new Uint8Array(exports.memory.buffer, ptr, len));
    exports.lume_free(ptr, len);
    const answer = JSON.parse(text) as
      { ok: true; value: unknown } | { ok: false; code: string; error: string };
    if (!answer.ok) throw new Refused(answer.code, answer.error);
    return answer.value;
  }
}
