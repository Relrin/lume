import { describe, expect, it } from "vitest";

import {
  LINK_SCHEMES,
  LINK_SCHEMES_TO_CONFIRM,
  PASTE_MAX_BYTES,
  PASTE_PREVIEW_CHARS,
  TITLE_MAX_BYTES,
} from "../src/limits.ts";
import { pasteDecision, pastePreview, ProgramClipboard } from "../src/terminal/clipboard.ts";
import { PromptStore } from "../src/terminal/prompts.ts";
import { sanitizeTitle } from "../src/terminal/title.ts";
import { DEFAULT_APPEARANCE, palette256 } from "../src/theme.ts";

const bytes = (text: string) => new TextEncoder().encode(text).length;

describe("a title a program set", () => {
  it("is shown as it is when it is harmless", () => {
    expect(sanitizeTitle("~/src — vim")).toBe("~/src — vim");
    expect(sanitizeTitle("")).toBe("");
  });

  it("loses its control characters", () => {
    expect(sanitizeTitle("a\x00b\x07c\x1b[31md\x7fe\x9bf")).toBe("abc[31mdef");
    expect(sanitizeTitle("line one\r\nline two\ttab")).toBe("line oneline twotab");
  });

  it("loses the characters that reorder or hide the text after them", () => {
    expect(sanitizeTitle("safe‮gnp.exe")).toBe("safegnp.exe");
    expect(sanitizeTitle("a⁦b⁩c‎d؜e")).toBe("abcde");
  });

  it("is cut at the limit", () => {
    const title = sanitizeTitle("x".repeat(TITLE_MAX_BYTES + 500));
    expect(title).toBe("x".repeat(TITLE_MAX_BYTES));
  });

  it("is cut on a character, never inside one", () => {
    // Each character is three bytes; the limit falls inside one.
    const title = sanitizeTitle("漢".repeat(TITLE_MAX_BYTES));
    expect(bytes(title)).toBeLessThanOrEqual(TITLE_MAX_BYTES);
    expect(bytes(title)).toBeGreaterThan(TITLE_MAX_BYTES - 3);
    expect([...title].every((character) => character === "漢")).toBe(true);
    // Two UTF-16 units, four bytes: neither half is left behind.
    const faces = sanitizeTitle("😀".repeat(TITLE_MAX_BYTES));
    expect([...faces].every((character) => character === "😀")).toBe(true);
    expect(bytes(faces)).toBe(TITLE_MAX_BYTES - (TITLE_MAX_BYTES % 4));
  });
});

describe("links", () => {
  it("are opened for web addresses and files only, and files only after asking", () => {
    expect([...LINK_SCHEMES]).toEqual(["http", "https", "file"]);
    expect(LINK_SCHEMES_TO_CONFIRM).toEqual(["file"]);
  });
});

describe("what happens to a paste", () => {
  it("is sent when the engine finds it safe", () => {
    expect(pasteDecision("ls -la", true, false)).toBe("send");
    expect(pasteDecision("ls -la", true, true)).toBe("send");
  });

  it("needs the user's say when it has a line break and the program does not bracket pastes", () => {
    expect(pasteDecision("echo one\necho two", false, false)).toBe("confirm");
  });

  it("is sent with its line breaks when the program brackets pastes", () => {
    expect(pasteDecision("echo one\necho two", false, true)).toBe("send");
  });

  it("needs the user's say when it could break out of the brackets", () => {
    expect(pasteDecision("harmless\x1b[201~rm -rf ~\n", false, true)).toBe("confirm");
    expect(pasteDecision("harmless\x1b[201~rm -rf ~\n", false, false)).toBe("confirm");
  });

  it("is refused above the size limit, whatever else is true of it", () => {
    const large = "x".repeat(PASTE_MAX_BYTES + 1);
    expect(pasteDecision(large, true, true)).toBe("too-large");
    expect(pasteDecision("x".repeat(PASTE_MAX_BYTES), true, false)).toBe("send");
    // The limit is on bytes: these are three each.
    expect(pasteDecision("漢".repeat(PASTE_MAX_BYTES / 2), true, false)).toBe("too-large");
  });
});

describe("the preview of a paste", () => {
  it("shows the text and counts its lines", () => {
    expect(pastePreview("one\ntwo\r\nthree")).toEqual({
      text: "one\ntwo\r\nthree",
      truncated: false,
      lines: 3,
    });
    expect(pastePreview("one").lines).toBe(1);
  });

  it("makes control characters visible and leaves line breaks and tabs alone", () => {
    expect(pastePreview("a\x1b[201~b\x00c\x7fd\te\n").text).toBe("a␛[201~b␀c␡d\te\n");
    expect(pastePreview("a\x9bb").text).toBe("a�b");
  });

  it("shows only the start of a long text and says so", () => {
    const preview = pastePreview("x".repeat(PASTE_PREVIEW_CHARS + 10));
    expect(preview.text).toHaveLength(PASTE_PREVIEW_CHARS);
    expect(preview.truncated).toBe(true);
    expect(pastePreview("x".repeat(PASTE_PREVIEW_CHARS)).truncated).toBe(false);
  });
});

describe("a program writing to the clipboard", () => {
  it("may write the ordinary clipboard and no other", () => {
    const clipboard = new ProgramClipboard();
    expect(clipboard.allows("standard", true)).toBe(true);
    expect(clipboard.allows("selection", true)).toBe(false);
    expect(clipboard.allows("primary", true)).toBe(false);
    // A build that ends in a pane nobody looks at may say so on the clipboard.
    expect(clipboard.allows("standard", false)).toBe(true);
  });

  it("may do so only with the keyboard when its terminal is a server's", () => {
    const clipboard = new ProgramClipboard(true);
    expect(clipboard.allows("standard", true)).toBe(true);
    expect(clipboard.allows("standard", false)).toBe(false);
    expect(clipboard.allows("selection", true)).toBe(false);
    // What was refused is not announced.
    expect(clipboard.noticeDue()).toBe(true);
  });

  it("is announced once for each session", () => {
    const first = new ProgramClipboard();
    expect([first.noticeDue(), first.noticeDue(), first.noticeDue()]).toEqual([true, false, false]);
    expect(new ProgramClipboard().noticeDue()).toBe(true);
  });
});

describe("prompts", () => {
  it("are shown one at a time, in the order they were raised", async () => {
    const store = new PromptStore();
    let changes = 0;
    const stop = store.subscribe(() => changes++);
    expect(store.current()).toBeNull();

    const paste = store.confirmPaste(pastePreview("a\nb"));
    const link = store.confirmLink("file:///tmp/x");
    store.notice("told");
    expect(changes).toBe(3);

    const first = store.current();
    expect(first?.kind).toBe("paste");
    // The same object until it is answered, so a component does not render in a loop.
    expect(store.current()).toBe(first);
    first?.answer(true);
    expect(await paste).toBe(true);

    const second = store.current();
    expect(second).toMatchObject({ kind: "link", target: "file:///tmp/x" });
    second?.answer(false);
    expect(await link).toBe(false);

    const third = store.current();
    expect(third).toMatchObject({ kind: "notice", message: "told" });
    third?.answer(true);
    expect(store.current()).toBeNull();
    expect(changes).toBe(6);

    stop();
    store.notice("unheard");
    expect(changes).toBe(6);
  });

  it("take one answer each", async () => {
    const store = new PromptStore();
    const asked = store.confirmLink("file:///tmp/x");
    const prompt = store.current();
    prompt?.answer(false);
    prompt?.answer(true);
    expect(await asked).toBe(false);
    expect(store.current()).toBeNull();
  });
});

describe("the built-in look", () => {
  it("has sixteen ANSI colours and makes a palette of 256 from them", () => {
    expect(DEFAULT_APPEARANCE.ansi).toHaveLength(16);
    const palette = palette256(DEFAULT_APPEARANCE.ansi);
    expect(palette).toHaveLength(256);
    expect(palette.slice(0, 16)).toEqual(DEFAULT_APPEARANCE.ansi);
    // The colour cube and the grey ramp are the same in every terminal.
    expect(palette[16]).toEqual({ r: 0, g: 0, b: 0 });
    expect(palette[196]).toEqual({ r: 255, g: 0, b: 0 });
    expect(palette[231]).toEqual({ r: 255, g: 255, b: 255 });
    expect(palette[232]).toEqual({ r: 8, g: 8, b: 8 });
    expect(palette[255]).toEqual({ r: 238, g: 238, b: 238 });
  });
});
