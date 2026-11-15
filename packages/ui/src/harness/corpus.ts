/**
 * The corpus suite: every recorded terminal session is replayed through the terminal
 * engine in this webview, and the screen it leaves is compared with the recording's
 * expected screen. The same recordings are replayed by the native engine and under Node;
 * this is the check that the engine behaves the same inside each webview.
 */

import { dump, parseScript, replay, type Vt } from "@lume/vt";

/** The files of every recording, by path. Built into the page: the suite reads no disk. */
const SCRIPTS = import.meta.glob<string>("../../../../tests/corpus/*/script.toml", {
  query: "?raw",
  import: "default",
  eager: true,
});
const EXPECTED = import.meta.glob<string>("../../../../tests/corpus/*/expect.txt", {
  query: "?raw",
  import: "default",
  eager: true,
});
const INPUTS = import.meta.glob<string>("../../../../tests/corpus/*/input.bin", {
  query: "?url",
  import: "default",
  eager: true,
});

/** How one recording fared. */
export interface RecordingResult {
  readonly name: string;
  readonly ok: boolean;
  /** Where the screen first differs from the expected one, when it does. */
  readonly problem?: string;
}

/** The suite's report. */
export interface CorpusReport {
  readonly ok: boolean;
  readonly recordings: readonly RecordingResult[];
}

/** The recording's directory name, from the path of one of its files. */
function nameOf(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 2] ?? path;
}

function sibling(files: Record<string, string>, name: string): string | undefined {
  const path = Object.keys(files).find((candidate) => nameOf(candidate) === name);
  return path === undefined ? undefined : files[path];
}

/**
 * The bytes behind the address the build gave a file. A small file is built into the
 * page as a `data:` address, which the page's content policy does not let it fetch, so
 * that form is decoded here.
 */
async function bytesAt(address: string): Promise<Uint8Array> {
  const marker = ";base64,";
  const at = address.indexOf(marker);
  if (address.startsWith("data:") && at >= 0) {
    const binary = atob(address.slice(at + marker.length));
    return Uint8Array.from(binary, (character) => character.charCodeAt(0));
  }
  const response = await fetch(address);
  if (!response.ok) throw new Error(`${address} could not be read (${response.status})`);
  return new Uint8Array(await response.arrayBuffer());
}

/** Line endings as the expected files have them in a checkout that converts them. */
function normalise(text: string): string {
  return text.replace(/\r\n/g, "\n");
}

/** The first line at which two screens differ, as a short description. */
function firstDifference(actual: string, expected: string): string {
  const got = actual.split("\n");
  const want = expected.split("\n");
  for (let line = 0; line < Math.max(got.length, want.length); line++) {
    if (got[line] !== want[line]) {
      return `line ${line + 1}: expected ${JSON.stringify(want[line] ?? null)}, got ${JSON.stringify(got[line] ?? null)}`;
    }
  }
  return "the screens differ";
}

async function check(vt: Vt, name: string): Promise<RecordingResult> {
  const script = sibling(SCRIPTS, name);
  const expected = sibling(EXPECTED, name);
  const input = sibling(INPUTS, name);
  if (script === undefined || expected === undefined || input === undefined) {
    return { name, ok: false, problem: "the recording is missing one of its files" };
  }
  try {
    const { terminal, replies } = replay(vt, parseScript(script), await bytesAt(input));
    try {
      const actual = normalise(dump(terminal, replies));
      const wanted = normalise(expected);
      return actual === wanted
        ? { name, ok: true }
        : { name, ok: false, problem: firstDifference(actual, wanted) };
    } finally {
      terminal.free();
    }
  } catch (error) {
    return { name, ok: false, problem: error instanceof Error ? error.message : String(error) };
  }
}

/** Replays every recording and reports how each fared. */
export async function runCorpus(vt: Vt, progress: (text: string) => void): Promise<CorpusReport> {
  const names = Object.keys(SCRIPTS).map(nameOf).sort();
  const recordings: RecordingResult[] = [];
  for (const name of names) {
    progress(`replaying ${name}`);
    recordings.push(await check(vt, name));
  }
  // A page built without its recordings must not pass by having nothing to fail.
  const ok = recordings.length > 0 && recordings.every((recording) => recording.ok);
  return { ok, recordings };
}
