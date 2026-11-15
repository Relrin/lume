/**
 * Shared set-up for the suites: the built module, and paths into the repository.
 */

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

import { Vt, type OutputKind, type Terminal } from "../src/index.ts";
import { Core, type Exports } from "../src/module.ts";

const wasmPath = fileURLToPath(new URL("../dist/ghostty-vt.wasm", import.meta.url));

/** The directory holding the golden recordings. */
export const corpusDir = fileURLToPath(new URL("../../../tests/corpus/", import.meta.url));

/** The bytes of the built module. */
export async function wasmBytes(): Promise<Uint8Array> {
  try {
    return await readFile(wasmPath);
  } catch {
    throw new Error(`${wasmPath} is missing. Run \`cargo xtask vt wasm\` first.`);
  }
}

/** A fresh module instance with nothing installed in it, for tests of the raw calls. */
export async function newCore(): Promise<Core> {
  const { instance } = await WebAssembly.instantiate((await wasmBytes()) as BufferSource, {});
  return new Core(instance.exports as unknown as Exports);
}

let shared: Promise<Vt> | undefined;

/** One module instance for a whole test file. */
export function loadVt(): Promise<Vt> {
  shared ??= wasmBytes().then((bytes) => Vt.instantiate(bytes));
  return shared;
}

/** What a terminal sent towards the pty, as text, in order. */
export interface Sent {
  replies: string[];
  input: string[];
  all: { kind: OutputKind; text: string }[];
}

/** Record everything `term` sends towards the pty from now on. */
export function recordOutput(term: Terminal): Sent {
  const sent: Sent = { replies: [], input: [], all: [] };
  const decoder = new TextDecoder();
  term.handlers = {
    ...term.handlers,
    output(bytes, kind) {
      const text = decoder.decode(bytes);
      sent.all.push({ kind, text });
      (kind === "reply" ? sent.replies : sent.input).push(text);
    },
  };
  return sent;
}
