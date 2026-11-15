import { readFileSync } from "node:fs";

import { Vt, type ClipboardLocation } from "@lume/vt";
import { beforeAll, describe, expect, it } from "vitest";

import { OSC52_WRITE_MAX_BYTES, SCROLLBACK_MAX_LINES } from "../src/limits.ts";
import { TerminalSession, type SessionHooks } from "../src/terminal/session.ts";
import { DEFAULT_APPEARANCE, terminalColors } from "../src/theme.ts";
import type { Attachment, StreamFrame } from "../src/transport/client.ts";
import type {
  BackendCaps,
  CellPx,
  ClientFacts,
  ExitStatus,
  Responder,
  TermSize,
} from "../src/transport/types.ts";

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

const ESC = "\x1b";
const CELL: CellPx = { w: 8, h: 16 };
const utf8 = (text: string) => new TextEncoder().encode(text);
const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

/** An attachment with a test at the other end instead of a session. */
class FakeAttachment implements Attachment {
  readonly size: TermSize = { cols: 80, rows: 24 };
  readonly resizePullsScrollback = false;
  readonly hydrate = "none" as const;
  readonly caps: BackendCaps;
  /** Everything the terminal's side did, in order. */
  readonly calls: string[] = [];
  readonly inputs: string[] = [];
  consumedBytes = 0;
  /** Sends a frame to the terminal, as the session would. */
  deliver: (frame: StreamFrame) => void = () => {
    throw new Error("the attachment has not been started");
  };

  constructor(responder: Responder = "client") {
    this.caps = {
      persistent: false,
      multiClient: false,
      serverLayout: false,
      responder,
      hydrate: { snapshot: false, vtReplay: false },
    };
  }

  start(receive: (frame: StreamFrame) => void): void {
    this.calls.push("start");
    this.deliver = receive;
  }
  input(bytes: Uint8Array): void {
    this.calls.push("input");
    this.inputs.push(text(bytes));
  }
  resize(size: TermSize, cellPx: CellPx): void {
    this.calls.push(`resize ${size.cols}x${size.rows} ${cellPx.w}x${cellPx.h}`);
  }
  facts(facts: ClientFacts): void {
    this.calls.push(`facts focused=${facts.focused}`);
  }
  consumed(bytes: number): void {
    this.consumedBytes += bytes;
  }
  detach(): void {
    this.calls.push("detach");
  }

  data(output: string): void {
    this.deliver({ stream: 1, kind: "data", bytes: utf8(output) });
  }
}

interface Opened {
  attachment: FakeAttachment;
  session: TerminalSession;
  /** What the session told its view, in order. */
  events: string[];
}

function open(responder: Responder = "client", hooks: SessionHooks = {}): Opened {
  const attachment = new FakeAttachment(responder);
  const events: string[] = [];
  const session = new TerminalSession(vt, attachment, {
    colors: terminalColors(DEFAULT_APPEARANCE),
    scheme: "dark",
    cellPx: CELL,
    hooks: {
      changed: () => events.push("changed"),
      resized: (size) => events.push(`resized ${size.cols}x${size.rows}`),
      exited: (status) => events.push(`exited ${JSON.stringify(status)}`),
      ...hooks,
    },
  });
  session.start();
  return { attachment, session, events };
}

const FACTS: ClientFacts = {
  size: { cols: 80, rows: 24 },
  cellPx: CELL,
  palette: {
    fg: DEFAULT_APPEARANCE.foreground,
    bg: DEFAULT_APPEARANCE.background,
    cursor: DEFAULT_APPEARANCE.cursor,
    ansi: DEFAULT_APPEARANCE.ansi,
  },
  scheme: "dark",
  focused: true,
  visible: true,
};

describe("a terminal session", () => {
  it("starts at the session's size", () => {
    const { session, attachment } = open();
    expect([session.terminal.cols, session.terminal.rows]).toEqual([80, 24]);
    expect(attachment.calls).toEqual(["start"]);
    session.dispose();
  });

  it("writes output into the terminal as it arrives and says it has been taken", () => {
    const { session, attachment, events } = open();
    attachment.data("hello\r\n");
    attachment.data(`${ESC}[31mred`);
    expect(session.terminal.rowText(0)).toBe("hello");
    expect(session.terminal.rowText(1)).toBe("red");
    expect(attachment.consumedBytes).toBe(utf8(`hello\r\n${ESC}[31mred`).length);
    expect(events).toEqual(["changed", "changed"]);
    session.dispose();
  });

  it("resizes the terminal at the point of the output where the session did", () => {
    const { session, attachment, events } = open();
    const line = "x".repeat(100);
    attachment.data(line);
    // Written at 80 columns, the line wrapped.
    expect(session.terminal.rowText(1)).toBe("x".repeat(20));
    attachment.deliver({ stream: 1, kind: "resized", size: { cols: 120, rows: 40 } });
    expect([session.terminal.cols, session.terminal.rows]).toEqual([120, 40]);
    attachment.data(`\r\n${"y".repeat(100)}`);
    // Written after the resize, the next one did not.
    expect(session.terminal.screenText().some((row) => row === "y".repeat(100))).toBe(true);
    expect(events).toEqual(["changed", "resized 120x40", "changed", "changed"]);
    session.dispose();
  });

  it("answers a program's questions when it is the one that answers", () => {
    const { session, attachment } = open("client");
    attachment.data(`${ESC}[c`);
    expect(attachment.inputs).toHaveLength(1);
    // The device attributes: an escape, "[?", numbers and a "c".
    const [attributes] = attachment.inputs;
    expect(attributes?.startsWith(`${ESC}[?`)).toBe(true);
    expect(attributes?.slice(3)).toMatch(/^[\d;]+c$/);
    // The cursor position, which only this terminal knows.
    attachment.data(`\r\n\r\nab${ESC}[6n`);
    expect(attachment.inputs[1]).toBe(`${ESC}[3;3R`);
    session.dispose();
  });

  it("keeps its answers to itself when the backend answers for the session", () => {
    const { session, attachment } = open("backend");
    attachment.data(`${ESC}[c${ESC}[6n${ESC}]11;?${ESC}\\`);
    expect(attachment.inputs).toEqual([]);
    // What the user does is sent whoever answers.
    session.send(utf8("ls\r"));
    expect(attachment.inputs).toEqual(["ls\r"]);
    session.dispose();
  });

  it("answers colour questions from the colours it was given", () => {
    const { session, attachment } = open();
    attachment.data(`${ESC}]11;?${ESC}\\`);
    const { r, g, b } = DEFAULT_APPEARANCE.background;
    const component = (value: number) => value.toString(16).padStart(2, "0").repeat(2);
    expect(attachment.inputs[0]).toContain(`rgb:${component(r)}/${component(g)}/${component(b)}`);
    session.dispose();
  });

  it("does not report its title back, which would let output type into the program", () => {
    const { session, attachment } = open();
    attachment.data(`${ESC}]0;rm -rf ~\x07${ESC}[21t`);
    expect(attachment.inputs).toEqual([]);
    session.dispose();
  });

  it("sends nothing for empty input", () => {
    const { session, attachment } = open();
    session.send(new Uint8Array(0));
    expect(attachment.inputs).toEqual([]);
    session.dispose();
  });

  it("asks for a size once, and not for the size the session already has", () => {
    const { session, attachment } = open();
    session.requestSize({ cols: 80, rows: 24 }, CELL);
    expect(attachment.calls).toEqual(["start"]);

    session.requestSize({ cols: 100, rows: 30 }, CELL);
    session.requestSize({ cols: 100, rows: 30 }, CELL);
    expect(attachment.calls).toEqual(["start", "resize 100x30 8x16"]);
    // The grid does not change until the session says so.
    expect(session.terminal.cols).toBe(80);

    // Cells of another size are news even on the same grid.
    session.requestSize({ cols: 100, rows: 30 }, { w: 16, h: 32 });
    expect(attachment.calls.at(-1)).toBe("resize 100x30 16x32");
    session.dispose();
  });

  it("passes facts on", () => {
    const { session, attachment } = open();
    session.facts(FACTS);
    session.facts({ ...FACTS, focused: false });
    expect(attachment.calls).toEqual(["start", "facts focused=true", "facts focused=false"]);
    session.dispose();
  });

  it("tells its view about the title and the bell", () => {
    const titles: string[] = [];
    let bells = 0;
    const { session, attachment } = open("client", {
      title: (title) => titles.push(title),
      bell: () => bells++,
    });
    attachment.data(`${ESC}]0;~/src${ESC}\\\x07`);
    expect(titles).toEqual(["~/src"]);
    expect(bells).toBe(1);
    session.dispose();
  });

  it("lets a program write text to the clipboard within the size limit", () => {
    const writes: [string, ClipboardLocation][] = [];
    const { session, attachment } = open("client", {
      clipboardWrite: (written, location) => writes.push([written, location]),
    });
    attachment.data(`${ESC}]52;c;${btoa("hello")}\x07`);
    expect(writes).toEqual([["hello", "standard"]]);

    const large = btoa("x".repeat(OSC52_WRITE_MAX_BYTES + 1));
    attachment.data(`${ESC}]52;c;${large}\x07`);
    expect(writes).toHaveLength(1);
    session.dispose();
  });

  it("never lets a program read the clipboard", () => {
    const { session, attachment } = open();
    attachment.data(`${ESC}]52;c;?\x07`);
    expect(attachment.inputs).toEqual([]);
    session.dispose();
  });

  it("keeps about as much scrollback as the limit and no more", () => {
    const { session, attachment } = open();
    const chunk = "x\r\n".repeat(1000);
    for (let i = 0; i < 25; i++) attachment.data(chunk);
    const kept = session.terminal.scrollbar().total;
    // The limit is applied a page of memory at a time.
    expect(kept).toBeGreaterThan(SCROLLBACK_MAX_LINES * 0.9);
    expect(kept).toBeLessThan(SCROLLBACK_MAX_LINES * 1.2);
    session.dispose();
  });

  it("replays output it is given to start from", () => {
    const { session, attachment, events } = open();
    attachment.deliver({ stream: 1, kind: "hydrate", form: "vt_replay", bytes: utf8("restored") });
    expect(session.terminal.rowText(0)).toBe("restored");
    expect(events).toEqual(["changed"]);
    // That is not output the session is waiting to hear was taken.
    expect(attachment.consumedBytes).toBe(0);
    session.dispose();
  });

  describe("once its program has ended", () => {
    function ended(status: ExitStatus = { code: 3 }): Opened {
      const opened = open();
      opened.attachment.data("last words");
      opened.attachment.deliver({ stream: 1, kind: "exit", status });
      return opened;
    }

    it("says so, once, and keeps the screen", () => {
      const { session, events } = ended();
      expect(session.exit).toEqual({ code: 3 });
      expect(events).toEqual(["changed", 'exited {"code":3}', "changed"]);
      expect(session.terminal.rowText(0)).toBe("last words");
      session.dispose();
    });

    it("sends the program nothing more", () => {
      const { session, attachment } = ended();
      session.send(utf8("x"));
      session.facts(FACTS);
      expect(attachment.inputs).toEqual([]);
      expect(attachment.calls).toEqual(["start"]);
      session.dispose();
    });

    it("follows the view's size by itself, with nobody left to ask", () => {
      const { session, attachment, events } = ended();
      session.requestSize({ cols: 100, rows: 30 }, CELL);
      expect([session.terminal.cols, session.terminal.rows]).toEqual([100, 30]);
      expect(attachment.calls).toEqual(["start"]);
      expect(events.at(-2)).toBe("resized 100x30");
      session.dispose();
    });
  });

  it("detaches when disposed and ignores what arrives afterwards", () => {
    const { session, attachment, events } = open();
    session.dispose();
    session.dispose();
    expect(attachment.calls).toEqual(["start", "detach"]);
    attachment.data("late");
    session.send(utf8("x"));
    session.requestSize({ cols: 100, rows: 30 }, CELL);
    expect(events).toEqual([]);
    expect(attachment.calls).toEqual(["start", "detach"]);
  });
});
