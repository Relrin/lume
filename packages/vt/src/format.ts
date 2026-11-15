/**
 * Terminal contents as text: for copying a selection and for capturing the screen.
 */

import type { Terminal } from "./terminal.ts";

/** The form the text takes. */
export type TextFormat = "plain" | "vt" | "html";

/** What to format and how. */
export interface FormatOptions {
  /** Defaults to `"plain"`. */
  format?: TextFormat;
  /** Only the selected text. With nothing selected the result is empty. */
  selection?: boolean;
  /** Join lines the terminal wrapped back into the lines the program wrote. */
  unwrap?: boolean;
  /** Remove trailing blanks. */
  trim?: boolean;
}

/** The contents of a terminal, or of its selection, as text. */
export function formatText(terminal: Terminal, options: FormatOptions = {}): string {
  const core = terminal.core;
  const { x } = core;
  const m = core.manifest;
  const option = (name: string) => m.enumValue("GhosttyFormatterOption", name);

  const formatter = core.newHandle("ghostty_formatter_terminal_new", (out) =>
    x.ghostty_formatter_terminal_new(0, out, terminal.handle),
  );
  try {
    const set = (name: string, value: number) =>
      core.check(x.ghostty_formatter_set(formatter, option(name), value), "ghostty_formatter_set");
    const setBool = (name: string, value: boolean) => {
      const at = core.scratch(4);
      core.view().setUint8(at, value ? 1 : 0);
      set(name, at);
    };

    if (options.selection) {
      const selection = core.scratch(m.struct("GhosttySelection").size);
      if (!terminal.getData("SELECTION", selection)) return "";
      // The formatter copies the selection, so the scratch memory can be reused.
      set("SELECTION", selection);
    }
    const format = core.scratch(4);
    const name = (options.format ?? "plain").toUpperCase();
    core.view().setInt32(format, m.enumValue("GhosttyFormatterFormat", name), true);
    set("EMIT", format);
    if (options.unwrap !== undefined) setBool("UNWRAP", options.unwrap);
    if (options.trim !== undefined) setBool("TRIM", options.trim);

    // Two words: where the library put the text, and how long it is.
    const out = core.scratch(8);
    core.check(
      x.ghostty_formatter_format_alloc(formatter, 0, out, out + 4),
      "ghostty_formatter_format_alloc",
    );
    const view = core.view();
    const ptr = view.getUint32(out, true);
    const len = view.getUint32(out + 4, true);
    try {
      return core.string(ptr, len);
    } finally {
      // The buffer is the library's own allocation, released through the library.
      x.ghostty_free(0, ptr, len);
    }
  } finally {
    x.ghostty_formatter_free(formatter);
  }
}

/**
 * The selected text as it should go on the clipboard: wrapped lines joined and trailing
 * blanks removed. Empty when nothing is selected.
 */
export function selectionText(terminal: Terminal): string {
  return formatText(terminal, { selection: true, unwrap: true, trim: true });
}
