/**
 * The golden corpus: recorded terminal byte streams with expected screen dumps.
 *
 * A recording is the bytes a program wrote (`input.bin`), a script giving the grid size
 * and the resizes to apply at byte offsets of the input (`script.toml`), and the dump the
 * screen must produce afterwards (`expect.txt`).
 *
 * The native build of the library replays the same recordings and writes the same dump
 * format. The two must agree byte for byte, so the dump here reads cells one at a time
 * exactly as the native one does, and is deliberately plain text.
 *
 * Nothing here touches the file system: a caller in Node or in a webview supplies the
 * bytes.
 */

import type { Attrs, Color } from "./cell.ts";
import type { Vt } from "./module.ts";
import type { Terminal } from "./terminal.ts";

/** `script.toml` of a recording. */
export interface Script {
  /** One line saying what the recording exercises. */
  description: string;
  cols: number;
  rows: number;
  /** Resizes, in order of `at`. */
  resize: Resize[];
}

/** A resize applied when `at` bytes of the input have been fed. */
export interface Resize {
  at: number;
  cols: number;
  rows: number;
}

/**
 * Parse a recording script.
 *
 * Scripts use a small part of TOML: comments, `key = "string"`, `key = integer` and
 * `[[resize]]` tables. Anything else, an unknown key or a missing one is an error, so a
 * script cannot say something this reader silently ignores.
 */
export function parseScript(text: string): Script {
  const top: Record<string, string | number> = {};
  const resizes: Record<string, string | number>[] = [];
  let table = top;

  text.split(/\r?\n/).forEach((raw, index) => {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) return;
    const fail = (why: string): never => {
      throw new Error(`script line ${index + 1}: ${why}`);
    };

    if (line.startsWith("[")) {
      if (line !== "[[resize]]") fail(`unknown table ${line}`);
      table = {};
      resizes.push(table);
      return;
    }

    const match = /^([A-Za-z_][A-Za-z0-9_-]*)\s*=\s*(.+)$/.exec(line);
    if (!match) return fail("expected `key = value`");
    const key = match[1]!;
    const value = match[2]!.trim();
    if (key in table) fail(`${key} is set twice`);

    if (/^[0-9]+$/.test(value)) {
      table[key] = Number(value);
    } else if (/^"(?:[^"\\]|\\.)*"$/.test(value)) {
      table[key] = unescape(value.slice(1, -1), fail);
    } else {
      fail(`unsupported value for ${key}`);
    }
  });

  const take = (
    from: Record<string, string | number>,
    allowed: Record<string, "string" | "number">,
  ) => {
    for (const key of Object.keys(from)) {
      if (!(key in allowed)) throw new Error(`script: unknown key ${key}`);
    }
    for (const [key, type] of Object.entries(allowed)) {
      if (typeof from[key] !== type) throw new Error(`script: ${key} must be a ${type}`);
    }
    return from;
  };

  const head = take(top, { description: "string", cols: "number", rows: "number" });
  return {
    description: head.description as string,
    cols: head.cols as number,
    rows: head.rows as number,
    resize: resizes.map((entry) => {
      const r = take(entry, { at: "number", cols: "number", rows: "number" });
      return { at: r.at as number, cols: r.cols as number, rows: r.rows as number };
    }),
  };
}

function unescape(body: string, fail: (why: string) => never): string {
  const escapes: Record<string, string> = { n: "\n", t: "\t", r: "\r", '"': '"', "\\": "\\" };
  return body.replace(
    /\\(.)/g,
    (_, ch: string) => escapes[ch] ?? fail(`unsupported escape \\${ch}`),
  );
}

/** A recording that has been fed to a terminal. */
export interface Replayed {
  /** The terminal as the recording left it. The caller frees it. */
  readonly terminal: Terminal;
  /** Every byte the terminal wrote back to the program while it was fed, in order. */
  readonly replies: Uint8Array;
}

/**
 * Feed a recording into a fresh terminal, applying each resize at its offset. The
 * terminal runs with the library's defaults, as the native replay does, and what it
 * answers to the program's questions is collected. The caller frees the terminal.
 */
export function replay(vt: Vt, script: Script, input: Uint8Array): Replayed {
  let previous = 0;
  for (const r of script.resize) {
    if (r.at < previous || r.at > input.length) {
      throw new Error(
        `resize offsets must be ascending and within the input (${input.length} bytes)`,
      );
    }
    previous = r.at;
  }

  const term = vt.createTerminal(script.cols, script.rows);
  const answered: Uint8Array[] = [];
  term.handlers = { output: (bytes) => void answered.push(bytes) };
  try {
    let fed = 0;
    for (const r of script.resize) {
      term.write(input.subarray(fed, r.at));
      term.resize(r.cols, r.rows);
      fed = r.at;
    }
    term.write(input.subarray(fed));
  } catch (err) {
    term.free();
    throw err;
  }
  term.handlers = {};

  const replies = new Uint8Array(answered.reduce((sum, piece) => sum + piece.length, 0));
  let at = 0;
  for (const piece of answered) {
    replies.set(piece, at);
    at += piece.length;
  }
  return { terminal: term, replies };
}

/**
 * The DEC private modes a dump reports when they are on, by the number a program uses:
 * cursor keys, origin, wraparound, cursor shown, the mouse reporting modes and formats,
 * focus events, alternate scroll, the alternate screen, bracketed paste, synchronized
 * output, grapheme clusters, colour scheme reports, and size reports.
 */
export const DEC_MODES: readonly number[] = [
  1, 6, 7, 25, 1000, 1002, 1003, 1004, 1006, 1007, 1016, 1049, 2004, 2026, 2027, 2031, 2048,
];

/** The ANSI modes a dump reports when they are on: insert, and linefeed/new line. */
export const ANSI_MODES: readonly number[] = [4, 20];

const encoder = new TextEncoder();

/**
 * Bytes as dump text: printable ASCII as it is, everything else and the backslash as
 * `\xNN`.
 */
function escape(bytes: Uint8Array): string {
  let out = "";
  for (const byte of bytes) {
    out +=
      byte >= 0x20 && byte <= 0x7e && byte !== 0x5c
        ? String.fromCharCode(byte)
        : `\\x${byte.toString(16).padStart(2, "0")}`;
  }
  return out;
}

/**
 * The screen dump of a terminal's active area.
 *
 * ```text
 * size 80x24
 * cursor 3,1 visible
 * screen primary
 * --- text
 * 00|hello
 * 01|red
 * ...
 * --- attrs
 * 01 0..3 fg=palette:1
 * --- state
 * title=vim
 * pwd=file://host/tmp
 * modes 7 25 2004
 * ansi-modes
 * --- replies
 * \x1b[?62;22c
 * ```
 *
 * Text rows have trailing blanks removed. The attrs section lists runs of adjacent cells
 * that share a style other than the default; `a..b` is a half-open column range.
 *
 * The state section gives the title and working directory the program set, and which of
 * the modes in {@link DEC_MODES} and {@link ANSI_MODES} are on. The replies section is
 * one line: every byte the terminal wrote back to the program, as `replies` holds them.
 * Bytes outside printable ASCII, and the backslash, are written as `\xNN`.
 */
export function dump(term: Terminal, replies: Uint8Array = new Uint8Array()): string {
  let out = `size ${term.cols}x${term.rows}\n`;

  const cursor = term.cursor();
  out += `cursor ${cursor.x},${cursor.y} ${cursor.visible ? "visible" : "hidden"}`;
  if (cursor.pendingWrap) out += " pending-wrap";
  out += `\nscreen ${term.alternateScreen() ? "alternate" : "primary"}\n`;

  let attrs = "";
  out += "--- text\n";
  for (let row = 0; row < term.rows; row++) {
    const label = String(row).padStart(2, "0");
    let line = "";
    let runStart = 0;
    let runStyle = "";
    const flush = (end: number) => {
      if (runStyle !== "") attrs += `${label} ${runStart}..${end} ${runStyle}\n`;
    };

    for (let col = 0; col < term.cols; col++) {
      const cell = term.cell(col, row);
      if (cell.wide !== "spacer-tail" && cell.wide !== "spacer-head") {
        line += cell.text === "" ? " " : cell.text;
      }
      const style = describeStyle(cell.fg, cell.bg, cell.attrs);
      if (style !== runStyle) {
        flush(col);
        runStart = col;
        runStyle = style;
      }
    }
    flush(term.cols);
    out += `${label}|${line.replace(/ +$/, "")}\n`;
  }
  out += `--- attrs\n${attrs}`;

  out += "--- state\n";
  out += `title=${escape(encoder.encode(term.title()))}\n`;
  out += `pwd=${escape(encoder.encode(term.pwd()))}\n`;
  const on = (modes: readonly number[], ansi: boolean) =>
    modes
      .filter((mode) => term.mode(mode, ansi))
      .map((mode) => ` ${mode}`)
      .join("");
  out += `modes${on(DEC_MODES, false)}\n`;
  out += `ansi-modes${on(ANSI_MODES, true)}\n`;
  return `${out}--- replies\n${escape(replies)}\n`;
}

/** A stable, space-separated description of a non-default style; empty for the default. */
function describeStyle(fg: Color, bg: Color, attrs: Attrs): string {
  const parts: string[] = [];
  const fgText = describeColor(fg);
  if (fgText) parts.push(`fg=${fgText}`);
  const bgText = describeColor(bg);
  if (bgText) parts.push(`bg=${bgText}`);
  if (attrs.bold) parts.push("bold");
  if (attrs.faint) parts.push("faint");
  if (attrs.italic) parts.push("italic");
  if (attrs.inverse) parts.push("inverse");
  if (attrs.strikethrough) parts.push("strikethrough");
  if (attrs.underline !== 0) parts.push(`underline:${attrs.underline}`);
  return parts.join(" ");
}

function describeColor(color: Color): string | undefined {
  switch (color.kind) {
    case "default":
      return undefined;
    case "palette":
      return `palette:${color.index}`;
    case "rgb": {
      const hex = (n: number) => n.toString(16).padStart(2, "0");
      return `rgb:${hex(color.r)}${hex(color.g)}${hex(color.b)}`;
    }
  }
}
