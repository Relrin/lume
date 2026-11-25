/**
 * A session's terminal given what nobody in their right mind would send: output whose
 * effects fail, copies of a screen that cannot be loaded, replays that hold requests,
 * sizes no screen has, and an engine with no room left.
 */

import { readFileSync } from "node:fs";

import { Vt, type Terminal } from "@lume/vt";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { SERVED_TERMINALS_MAX, TERMINAL_CELLS_MAX } from "../src/limits.ts";
import { ServedBudget } from "../src/terminal/budget.ts";
import {
  TerminalSession,
  type SessionHooks,
  type SessionOptions,
} from "../src/terminal/session.ts";
import { DEFAULT_APPEARANCE, terminalColors } from "../src/theme.ts";
import type { HydrateFrame } from "../src/transport/client.ts";
import type { TermSize } from "../src/transport/types.ts";
import { FakeAttachment } from "./fake-attachment.ts";

const ESC = "\x1b";
const CELL = { w: 8, h: 16 };
const utf8 = (text: string) => new TextEncoder().encode(text);
/** Everything a program can ask of the terminal besides drawing. */
const REQUESTS =
  `${ESC}]52;c;${btoa("stolen")}\x07` +
  `${ESC}]0;a title\x07\x07` +
  `${ESC}[c${ESC}[6n${ESC}]11;?${ESC}\\`;

let engine: Vt;
/** The engine, as the session is given it: what it makes is kept note of. */
let vt: Vt;
const made: Terminal[] = [];
/** What is done to the next terminals that are made, each in its turn. */
const spoil: ((terminal: Terminal) => void)[] = [];

beforeAll(async () => {
  engine = await Vt.instantiate(
    readFileSync(new URL("../../vt/dist/ghostty-vt.wasm", import.meta.url)),
  );
});

beforeEach(() => {
  made.length = 0;
  spoil.length = 0;
  vi.spyOn(console, "warn").mockImplementation(() => undefined);
  const noted = (terminal: Terminal): Terminal => {
    made.push(terminal);
    spoil.shift()?.(terminal);
    return terminal;
  };
  vt = Object.create(engine, {
    createTerminal: {
      value: (...args: Parameters<Vt["createTerminal"]>) => noted(engine.createTerminal(...args)),
    },
    loadTerminal: {
      value: (...args: Parameters<Vt["loadTerminal"]>) => noted(engine.loadTerminal(...args)),
    },
  }) as Vt;
});

afterEach(() => {
  vi.restoreAllMocks();
});

/** The terminals that were made and not freed. */
const alive = (): Terminal[] => made.filter((terminal) => terminal.handle !== 0);

/** Has the engine say there is no room for another of something it makes. */
function noRoomFor(what: "ghostty_render_state_new", run: () => void): void {
  const core = engine.core as unknown as { x: Record<string, unknown> };
  const real = core.x;
  core.x = { ...real, [what]: () => -1 };
  try {
    run();
  } finally {
    core.x = real;
  }
}

function open(
  hooks: SessionHooks = {},
  options: Partial<SessionOptions> = {},
  attachment = new FakeAttachment("client", true),
) {
  const session = new TerminalSession(vt, attachment, {
    colors: terminalColors(DEFAULT_APPEARANCE),
    scheme: "dark",
    cellPx: CELL,
    hooks,
    ...options,
  });
  session.start();
  return { attachment, session };
}

const replay = (text: string): HydrateFrame => ({
  stream: 1,
  kind: "hydrate",
  form: "vt_replay",
  bytes: utf8(text),
});
const snapshot = (bytes: Uint8Array, vtVersion = engine.version): HydrateFrame => ({
  stream: 1,
  kind: "hydrate",
  form: "snapshot",
  vtVersion,
  bytes,
});
const DAMAGED = new Uint8Array([1, 2, 3]);
const saved = () =>
  new Uint8Array(
    readFileSync(new URL("../../../tests/corpus/07-input-modes/snapshot.bin", import.meta.url)),
  );
const sizeOf = (terminal: Terminal): TermSize => ({ cols: terminal.cols, rows: terminal.rows });

describe("output whose effects fail", () => {
  it("is granted back all the same, and what follows is shown", () => {
    let failing = true;
    const { attachment, session } = open({
      clipboardWrite: () => {
        if (failing) throw new Error("no clipboard today");
      },
    });
    const bad = `one${ESC}]52;c;${btoa("x")}\x07`;
    expect(() => attachment.data(bad)).toThrow("no clipboard today");
    expect(attachment.consumedBytes).toBe(bad.length);
    failing = false;
    attachment.data(" two");
    expect(session.terminal.rowText(0)).toBe("one two");
    expect(attachment.consumedBytes).toBe(bad.length + 4);
    session.dispose();
    expect(alive()).toEqual([]);
  });
});

describe("a replay", () => {
  const heard: string[] = [];
  const hooks: SessionHooks = {
    clipboardWrite: (text) => heard.push(`clipboard ${text}`),
    bell: () => heard.push("bell"),
    title: (title) => heard.push(`title ${title}`),
  };
  beforeEach(() => {
    heard.length = 0;
  });

  it("asks for nothing and answers nothing when the terminal starts from it", () => {
    const { attachment, session } = open(hooks, { start: replay(`shown${REQUESTS}`) });
    expect(session.terminal.rowText(0)).toBe("shown");
    expect(heard).toEqual([]);
    expect(attachment.inputs).toEqual([]);
    // What the program prints from here on is its own asking.
    attachment.data(REQUESTS);
    expect(heard).toEqual(["clipboard stolen", "title a title", "bell"]);
    expect(attachment.inputs).toHaveLength(3);
    session.dispose();
  });

  it("asks for nothing and answers nothing in the middle of the flow", () => {
    const { attachment, session } = open(hooks);
    for (let again = 0; again < 3; again++) attachment.deliver(replay(`shown${REQUESTS}`));
    expect(session.terminal.rowText(0)).toBe("shown");
    expect(alive()).toEqual([session.terminal]);
    expect(heard).toEqual([]);
    expect(attachment.inputs).toEqual([]);
    attachment.data(REQUESTS);
    expect(heard).toEqual(["clipboard stolen", "title a title", "bell"]);
    expect(attachment.inputs).toHaveLength(3);
    session.dispose();
  });

  it("leaves a terminal loaded from a snapshot able to ask", () => {
    const { attachment, session } = open(hooks, { start: snapshot(saved()) });
    attachment.data(REQUESTS);
    expect(heard).toEqual(["clipboard stolen", "title a title", "bell"]);
    session.dispose();
  });
});

describe("a copy of the screen that cannot be shown", () => {
  it("is said to be so when the terminal starts with it, and the terminal starts blank", () => {
    for (const start of [snapshot(DAMAGED), snapshot(saved(), "0.0.0-another")]) {
      const { attachment, session } = open({}, { start });
      expect(session.problem).toMatch(/could not be shown/);
      attachment.data("from here on");
      expect(session.terminal.rowText(0)).toBe("from here on");
      session.dispose();
    }
    expect(alive()).toEqual([]);
  });

  it("is said to be so in the middle of the flow, and the terminal stays as it is", () => {
    for (const frame of [snapshot(DAMAGED), snapshot(saved(), "0.0.0-another")]) {
      const told: string[] = [];
      let changed = 0;
      const { attachment, session } = open({
        problem: (message) => told.push(message),
        changed: () => changed++,
      });
      attachment.data("kept");
      const before = session.terminal;
      changed = 0;
      attachment.deliver(frame);
      attachment.deliver(frame);
      expect(session.terminal).toBe(before);
      expect(session.terminal.rowText(0)).toBe("kept");
      // Said once, however often it comes.
      expect(told).toHaveLength(1);
      expect(session.problem).toBe(told[0]);
      expect(changed).toBeGreaterThan(0);
      session.dispose();
    }
    expect(alive()).toEqual([]);
  });

  it("leaves nothing of the half that was made", () => {
    const { attachment, session } = open();
    attachment.data("kept");
    const before = session.terminal;
    const refuse = (method: "write" | "resize") => (terminal: Terminal) => {
      terminal[method] = () => {
        throw new Error(`no ${method}`);
      };
    };
    spoil.push(refuse("write"));
    attachment.deliver(replay("never"));
    // A loaded terminal has another size, and is resized.
    attachment.deliver({ stream: 1, kind: "resized", size: { cols: 100, rows: 30 } });
    spoil.push(refuse("resize"));
    attachment.deliver(snapshot(saved()));
    expect(session.terminal).toBe(before);
    expect(alive()).toEqual([before]);
    noRoomFor("ghostty_render_state_new", () => attachment.deliver(replay("never")));
    expect(session.terminal).toBe(before);
    expect(alive()).toEqual([before]);
    // The terminal that stayed is whole.
    attachment.data(" and on");
    expect(session.terminal.rowText(0)).toBe("kept and on");
    expect(session.state.update().cols).toBe(100);
    session.dispose();
    expect(alive()).toEqual([]);
  });

  it("is said no longer once one has been shown", () => {
    const told: string[] = [];
    const { attachment, session } = open({ problem: (message) => told.push(message) });
    session.release();
    session.restore();
    attachment.deliver(snapshot(DAMAGED));
    expect(session.restoring).toBe(false);
    expect(session.problem).not.toBe("");
    attachment.deliver(replay("there it is"));
    expect(session.problem).toBe("");
    expect(told.at(-1)).toBe("");
    session.dispose();
  });

  it("is not asked for again and again when it held the older lines", () => {
    const { attachment, session } = open({}, { windowLines: 200 });
    for (let line = 0; line < 1000; line++) attachment.data(`line ${line}\r\n`);
    session.fetchHistory();
    attachment.deliver(snapshot(DAMAGED));
    expect(session.problem).not.toBe("");
    for (let frame = 0; frame < 10; frame++) session.fetchHistory();
    expect(attachment.calls.filter((call) => call === "rehydrate")).toHaveLength(1);
    // One that can be shown makes it worth asking.
    attachment.deliver(replay("shown"));
    session.fetchHistory();
    expect(attachment.calls.filter((call) => call === "rehydrate")).toHaveLength(2);
    session.dispose();
  });
});

describe("an engine with no room left", () => {
  it("leaves no terminal and no attachment behind when a session cannot be made", () => {
    const attachment = new FakeAttachment("client", true);
    noRoomFor("ghostty_render_state_new", () => {
      expect(() => open({}, {}, attachment)).toThrow();
    });
    expect(alive()).toEqual([]);
    expect(attachment.calls).toEqual(["detach"]);
  });

  it("starts blank when there is room for that and not for what it would start from", () => {
    spoil.push((terminal) => {
      terminal.write = () => {
        throw new Error("no room");
      };
    });
    const { attachment, session } = open({}, { start: replay("x") });
    expect(session.problem).toMatch(/could not be shown/);
    expect(alive()).toEqual([session.terminal]);
    attachment.data("from here on");
    expect(session.terminal.rowText(0)).toBe("from here on");
    session.dispose();
  });

  it("keeps the screen when an empty one to let go of it for cannot be made", () => {
    const { attachment, session } = open();
    attachment.data("kept");
    noRoomFor("ghostty_render_state_new", () => {
      expect(() => session.release()).toThrow();
    });
    expect(session.released).toBe(false);
    expect(attachment.calls).not.toContain("release");
    expect(alive()).toEqual([session.terminal]);
    attachment.data(" and on");
    expect(session.terminal.rowText(0)).toBe("kept and on");
    session.dispose();
    expect(alive()).toEqual([]);
  });
});

describe("a size no screen has", () => {
  const HUGE: TermSize = { cols: 4096, rows: 4096 };
  const cells = (terminal: Terminal) => terminal.cols * terminal.rows;

  it("is brought within what a terminal may have, wherever it comes from", () => {
    const attachment = new FakeAttachment("client", true);
    Object.assign(attachment, { size: HUGE });
    const resized: TermSize[] = [];
    const { session } = open({ resized: (size) => resized.push(size) }, {}, attachment);
    expect(cells(session.terminal)).toBeLessThanOrEqual(TERMINAL_CELLS_MAX);
    attachment.deliver({ stream: 1, kind: "resized", size: { cols: 4096, rows: 200 } });
    expect(sizeOf(session.terminal)).toEqual({ cols: 2621, rows: 200 });
    expect(resized).toEqual([{ cols: 2621, rows: 200 }]);
    attachment.deliver({ stream: 1, kind: "resized", size: HUGE });
    expect(cells(session.terminal)).toBeLessThanOrEqual(TERMINAL_CELLS_MAX);
    // With nobody left to answer, the view's own wish is kept within it too.
    attachment.deliver({ stream: 1, kind: "exit", status: { code: 0 } });
    session.requestSize({ cols: 4096, rows: 4095 }, CELL);
    expect(cells(session.terminal)).toBeLessThanOrEqual(TERMINAL_CELLS_MAX);
    session.dispose();
  });

  it("is not taken from a copy of the screen either", () => {
    const { attachment, session } = open();
    spoil.push((terminal) => terminal.resize(2000, 2000));
    attachment.deliver(snapshot(saved()));
    expect(sizeOf(session.terminal)).toEqual({ cols: 80, rows: 24 });
    session.dispose();
  });
});

describe("a terminal of a server", () => {
  it("has the share of the window it was given, until it is gone", () => {
    const budget = new ServedBudget(() => 0);
    const share = budget.open()!;
    const { attachment, session } = open({}, { share });
    for (let others = 0; others < 4; others++) budget.open()!.fit({ cols: 4096, rows: 4096 });
    attachment.deliver({ stream: 1, kind: "resized", size: { cols: 4096, rows: 100 } });
    expect(session.terminal.cols).toBeLessThan(100);
    session.dispose();
    // Its place is free again.
    let left = 0;
    while (budget.open()) left++;
    expect(left).toBe(SERVED_TERMINALS_MAX - 4);
  });

  it("keeps no more scrollback than its share, whatever it prints", () => {
    const kept = (scrollbackBytes: number): number => {
      const share = { scrollbackBytes, fit: (size: TermSize) => size, close: () => undefined };
      const { attachment, session } = open({}, { share, scrollbackLines: 100_000 });
      const chunk = `${"x".repeat(79)}\r\n`.repeat(1000);
      for (let i = 0; i < 40; i++) attachment.data(chunk);
      const { total } = session.terminal.scrollbar();
      session.dispose();
      return total;
    };
    const little = kept(2 * 1024 * 1024);
    expect(little).toBeLessThan(10_000);
    expect(kept(64 * 1024 * 1024)).toBeGreaterThan(little * 3);
  });

  it("lets go of its share when it cannot be made", () => {
    const closed = vi.fn();
    const share = { scrollbackBytes: 1024, fit: (size: TermSize) => size, close: closed };
    noRoomFor("ghostty_render_state_new", () => {
      expect(() => open({}, { share })).toThrow();
    });
    expect(closed).toHaveBeenCalledTimes(1);
  });
});
