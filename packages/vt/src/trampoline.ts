/**
 * Making JavaScript functions callable from inside the WebAssembly module.
 *
 * libghostty-vt reports effects (bytes for the pty, the bell, a new title) by calling C
 * function pointers. In WebAssembly a function pointer is an index into the module's
 * function table, and a table accepts only WebAssembly functions: a plain JavaScript
 * function cannot be stored in it.
 *
 * The way round is a second, tiny module per function signature that imports one function
 * and exports it again unchanged. Instantiating it with a JavaScript function as the
 * import yields an exported WebAssembly function with exactly that signature, which the
 * table does accept. Calls through the table then land in the JavaScript function.
 */

import type { Signature, ValType } from "./manifest.ts";

const VALUE_TYPE: Readonly<Record<ValType, number>> = {
  i32: 0x7f,
  i64: 0x7e,
  f32: 0x7d,
  f64: 0x7c,
};

const SECTION_TYPE = 1;
const SECTION_IMPORT = 2;
const SECTION_EXPORT = 7;
const FUNCTION_TYPE = 0x60;
const KIND_FUNCTION = 0;

/** The single letter both the import module and the export are named. */
const NAME = 0x66;

/** An unsigned integer in the variable-length encoding the binary format uses. */
function leb128(value: number): number[] {
  const bytes: number[] = [];
  do {
    const low = value & 0x7f;
    value >>>= 7;
    bytes.push(value === 0 ? low : low | 0x80);
  } while (value !== 0);
  return bytes;
}

function section(id: number, body: number[]): number[] {
  return [id, ...leb128(body.length), ...body];
}

function valueTypes(types: readonly ValType[]): number[] {
  return [...leb128(types.length), ...types.map((type) => VALUE_TYPE[type])];
}

/**
 * The bytes of a module that imports one function of the given signature and exports
 * it again.
 */
export function thunkModule(signature: Signature): Uint8Array<ArrayBuffer> {
  return new Uint8Array([
    // Magic number and version 1.
    ...[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
    // One function type.
    ...section(SECTION_TYPE, [
      1,
      FUNCTION_TYPE,
      ...valueTypes(signature.params),
      ...valueTypes(signature.results),
    ]),
    // One import of that type: module "f", field "f".
    ...section(SECTION_IMPORT, [1, 1, NAME, 1, NAME, KIND_FUNCTION, 0]),
    // The import, exported as "f". Imported functions come first in the index space.
    ...section(SECTION_EXPORT, [1, 1, NAME, KIND_FUNCTION, 0]),
  ]);
}

/** Any function: thunks pass their arguments through untouched. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type Callback = (...args: any[]) => unknown;

/** Installs JavaScript functions into a module's function table. */
export class Trampoline {
  readonly #table: WebAssembly.Table;
  /** Compiled thunk modules by signature: compiling is the expensive part. */
  readonly #modules = new Map<string, WebAssembly.Module>();
  /** Table slots that were released and can be used again. */
  readonly #free: number[] = [];

  constructor(table: WebAssembly.Table) {
    this.#table = table;
  }

  /**
   * Make `fn` callable through the table with the given signature.
   *
   * Returns the table index, which is the value C code sees as the function pointer. A
   * 64-bit parameter or result is a `bigint` on the JavaScript side.
   */
  install(signature: Signature, fn: Callback): number {
    const key = `${signature.params.join(",")}>${signature.results.join(",")}`;
    let module = this.#modules.get(key);
    if (!module) {
      module = new WebAssembly.Module(thunkModule(signature));
      this.#modules.set(key, module);
    }
    const thunk = new WebAssembly.Instance(module, { f: { f: fn } }).exports.f;

    let index = this.#free.pop();
    if (index === undefined) {
      index = this.#table.length;
      this.#table.grow(1);
    }
    this.#table.set(index, thunk);
    return index;
  }

  /** The WebAssembly function at a table index, to call a function pointer the module gave us. */
  get(index: number): Callback {
    const fn: unknown = this.#table.get(index);
    if (typeof fn !== "function") throw new Error(`function table entry ${index} is empty`);
    return fn as Callback;
  }

  /**
   * Release a slot returned by `install`. The module must no longer hold the index: a
   * call through a released slot traps.
   */
  release(index: number): void {
    this.#table.set(index, null);
    this.#free.push(index);
  }
}
