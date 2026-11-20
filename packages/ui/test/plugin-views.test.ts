import { describe, expect, it } from "vitest";

import type { KeyBinding, OpenerDef } from "../src/core/types.ts";
import {
  forwardedEvent,
  mayForward,
  worthForwarding,
  type ForwardedKey,
} from "../src/keys/forwarded.ts";
import { Shortcuts } from "../src/keys/shortcuts.ts";
import {
  FrameBridge,
  type FrameHost,
  type PortLike,
  readMessage,
  STATE_MAX,
} from "../src/plugins/frame/bridge.ts";
import { fits, openersFor } from "../src/plugins/openers.ts";
import { KeyLayout } from "../src/terminal/keyboard.ts";
import type { Carrier, CarrierOpened } from "../src/transport/carrier.ts";
import { type Pipe, TransportClient } from "../src/transport/client.ts";
import { HEADER_LEN, Kind } from "../src/transport/frames.ts";
import {
  FrameStreams,
  type RelayClock,
  STREAM_BYTES_BURST,
  StreamRelay,
  type StreamPort,
  STREAMS_PER_FRAME,
  WRITE_MAX,
} from "../src/transport/view-streams.ts";
import { LIMITS, rawFrame } from "./support.ts";

class FakePort implements PortLike, StreamPort {
  sent: { message: unknown; transfer: Transferable[] | undefined }[] = [];
  closed = false;
  onmessage: ((event: MessageEvent) => void) | null = null;
  postMessage(message: unknown, transfer?: Transferable[]): void {
    this.sent.push({ message, transfer });
  }
  close(): void {
    this.closed = true;
  }
  receive(data: unknown): void {
    this.onmessage?.({ data } as MessageEvent);
  }
  messages(): unknown[] {
    return this.sent.map((one) => one.message);
  }
}

const settle = async () => {
  for (let i = 0; i < 5; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
};

function host(overrides: Partial<FrameHost> = {}) {
  const seen = { states: [] as unknown[], keys: [] as unknown[], stopped: [] as string[] };
  const made: FrameHost = {
    call: async () => null,
    ready: () => undefined,
    state: (state) => void seen.states.push(state),
    key: (key) => void seen.keys.push(key),
    stream: () => Promise.reject(Object.assign(new Error("no such grant"), { code: "not_found" })),
    stop: (reason) => void seen.stopped.push(reason),
    ...overrides,
  };
  return { made, seen };
}

describe("what a view keeps and opens", () => {
  it("keeps what fits and says so when it does not", () => {
    const port = new FakePort();
    const { made, seen } = host();
    new FrameBridge(port, made, () => 0);
    port.receive({ t: "state", state: { file: "a.rs" } });
    port.receive({ t: "state", state: "x".repeat(STATE_MAX) });
    expect(seen.states).toEqual([{ file: "a.rs" }]);
    expect(port.messages()).toEqual([expect.objectContaining({ t: "refused", what: "state" })]);
    // Too much kept is no reason to stop the view.
    expect(seen.stopped).toEqual([]);
  });

  it("hands a view the port of a stream it opened, and the refusal of one it may not", async () => {
    const theirs = { stream: true } as unknown as Transferable;
    const asked: string[] = [];
    const port = new FakePort();
    const { made } = host({
      stream: (grant) => {
        asked.push(grant);
        return grant === "good"
          ? Promise.resolve({ port: theirs, readable: true, writable: false })
          : Promise.reject(
              Object.assign(new Error("there is no such grant"), { code: "not_found" }),
            );
      },
    });
    new FrameBridge(port, made, () => 0);
    port.receive({ t: "stream", id: 4, grant: "good" });
    port.receive({ t: "stream", id: 5, grant: "another-views" });
    await settle();
    expect(asked).toEqual(["good", "another-views"]);
    expect(port.sent[0]).toEqual({
      message: { t: "opened", id: 4, port: theirs, readable: true, writable: false },
      transfer: [theirs],
    });
    expect(port.sent[1]?.message).toEqual({
      t: "error",
      id: 5,
      code: "not_found",
      message: "there is no such grant",
    });
    for (const bad of [
      { t: "stream", id: 1 },
      { t: "stream", id: 1, grant: "" },
      { t: "stream", id: 1, grant: "g".repeat(65) },
      { t: "stream", id: -1, grant: "g" },
    ]) {
      expect(readMessage(bad)).toBeNull();
    }
  });
});

function binding(keys: string, command: string, when = ""): KeyBinding {
  return {
    keys: keys.split(" "),
    command,
    args: null,
    when: when === "" ? [] : when.split(" && "),
    source: { layer: "default" },
  };
}

/** What a view's keys run, under bindings a user might have. */
function forwarding(...bindings: KeyBinding[]) {
  const ran: string[] = [];
  const shortcuts = new Shortcuts({ platform: "windows", layout: new KeyLayout() }, (command) => {
    if (mayForward(command)) ran.push(command);
  });
  shortcuts.setBindings(bindings);
  const press = (key: ForwardedKey): void => {
    if (worthForwarding(key)) shortcuts.handle(forwardedEvent(key), new Set());
  };
  return { ran, press };
}

describe("keys a view passes on", () => {
  const ctrlShift = (key: string, code: string): ForwardedKey => ({
    key,
    code,
    mods: ["ctrl", "shift"],
  });

  it("move about the window", () => {
    const { ran, press } = forwarding(
      binding("ctrl+shift+left", "lume.pane.focus-left"),
      binding("ctrl+tab", "lume.tab.next"),
      binding("ctrl+shift+p", "lume.palette.open"),
      binding("ctrl+shift+b", "lume.panel.toggle"),
    );
    press(ctrlShift("ArrowLeft", "ArrowLeft"));
    press({ key: "Tab", code: "Tab", mods: ["ctrl"] });
    press(ctrlShift("P", "KeyP"));
    press(ctrlShift("B", "KeyB"));
    expect(ran).toEqual([
      "lume.pane.focus-left",
      "lume.tab.next",
      "lume.palette.open",
      "lume.panel.toggle",
    ]);
  });

  it("never paste, type, close, start a program or run a plugin's command", () => {
    // A view may describe any key it likes, pressed or not, under whatever the user
    // has bound: none of these runs.
    const forged: [string, string][] = [
      ["ctrl+shift+v", "lume.terminal.paste"],
      ["ctrl+shift+c", "lume.terminal.copy"],
      ["ctrl+shift+w", "lume.pane.close"],
      ["ctrl+shift+t", "lume.tab.new"],
      ["ctrl+shift+q", "lume.app.quit"],
      ["ctrl+shift+n", "lume.window.new"],
      ["ctrl+shift+d", "lume.pane.split-right"],
      ["ctrl+shift+o", "lume.connect.open"],
      ["ctrl+shift+e", "lume.app.open"],
      ["ctrl+shift+g", "acme.tools.deploy"],
      ["ctrl+shift+k", "lume.terminal.clear"],
    ];
    const { ran, press } = forwarding(...forged.map(([keys, command]) => binding(keys, command)));
    for (const [keys] of forged) {
      const letter = keys.slice(-1);
      press(ctrlShift(letter.toUpperCase(), `Key${letter.toUpperCase()}`));
    }
    expect(ran).toEqual([]);
    for (const [, command] of forged) expect(mayForward(command)).toBe(false);
  });

  it("are not plain typing, and hold nowhere but in the window at large", () => {
    const { ran, press } = forwarding(
      binding("a", "lume.palette.open"),
      binding("ctrl+shift+p", "lume.palette.open", "terminal"),
      binding("f6", "lume.pane.focus-next"),
    );
    press({ key: "a", code: "KeyA", mods: [] });
    press({ key: "A", code: "KeyA", mods: ["shift"] });
    // Bound only where a terminal has the keyboard, which a view is not.
    press(ctrlShift("P", "KeyP"));
    expect(ran).toEqual([]);
    press({ key: "F6", code: "F6", mods: [] });
    expect(ran).toEqual(["lume.pane.focus-next"]);
    expect(worthForwarding({ key: "F25", code: "F25", mods: [] })).toBe(false);
  });
});

describe("what opens a link", () => {
  const openers: OpenerDef[] = [
    {
      plugin: "acme.review",
      view: "review",
      title: "Review",
      patterns: ["*.diff", "*.patch"],
      schemes: ["acme-review"],
    },
  ];

  it("fits names to patterns as the application does", () => {
    expect(fits("*.diff", "CHANGE.DIFF")).toBe(true);
    expect(fits("a*b*c", "a--b--b--c")).toBe(true);
    expect(fits("*", "")).toBe(true);
    expect(fits("*.diff", "change.diff.orig")).toBe(false);
    expect(fits("a*b", "a--c")).toBe(false);
    expect(fits("", "x")).toBe(false);
  });

  it("offers a view for a file by its name and for an address by its scheme", () => {
    const views = (link: string) => openersFor(link, openers).map((one) => one.view);
    expect(views("file:///C:/work/my%20fix.patch")).toEqual(["review"]);
    expect(views("file:///home/me/readme.md")).toEqual([]);
    expect(views("ACME-REVIEW://change/12")).toEqual(["review"]);
    // An address is matched by its scheme alone, never by how it ends.
    expect(views("https://example.com/a.diff")).toEqual([]);
    expect(views("not a link")).toEqual([]);
    expect(views("file:///C:/work/")).toEqual([]);
  });
});

/** A stream's end at the core, as a relay uses it. */
class FakePipe implements Pipe {
  readable = true;
  writable = true;
  receive: (bytes: Uint8Array) => void = () => undefined;
  ended: (ok: boolean) => void = () => undefined;
  consumedBytes = 0;
  written: Uint8Array[] = [];
  finished = 0;
  closed = 0;
  start(receive: (bytes: Uint8Array) => void, ended: (ok: boolean) => void): void {
    this.receive = receive;
    this.ended = ended;
  }
  consumed(bytes: number): void {
    this.consumedBytes += bytes;
  }
  write(bytes: Uint8Array): void {
    this.written.push(bytes);
  }
  finish(): void {
    this.finished += 1;
  }
  close(): void {
    this.closed += 1;
  }
}

/** A clock a test moves, with the tasks put off until it does. */
class FakeClock implements RelayClock {
  at = 0;
  tasks: { due: number; task: () => void }[] = [];
  /** How far the clock goes each time it is read, as work that takes time. */
  cost = 0;
  now(): number {
    this.at += this.cost;
    return this.at;
  }
  later(task: () => void, ms: number): void {
    this.tasks.push({ due: this.at + ms, task });
  }
  soon(task: () => void): void {
    this.tasks.push({ due: this.at, task });
  }
  /** Moves to `at` and runs what was due by then. */
  advance(at: number): void {
    this.at = at;
    for (;;) {
      const next = this.tasks.findIndex((one) => one.due <= this.at);
      if (next < 0) return;
      this.tasks.splice(next, 1)[0]?.task();
    }
  }
}

describe("a stream to a view", () => {
  function relay(clock = new FakeClock()) {
    const pipe = new FakePipe();
    const port = new FakePort();
    let gone = 0;
    const made = new StreamRelay(pipe, port, () => void (gone += 1), clock);
    return { pipe, port, made, clock, gone: () => gone };
  }

  it("hands over each piece as a buffer of its own, and credit as the view takes them", () => {
    const { pipe, port } = relay();
    const whole = new Uint8Array([9, 1, 2, 3, 9]);
    pipe.receive(whole.subarray(1, 4));
    expect(port.sent).toHaveLength(1);
    const { message, transfer } = port.sent[0]!;
    const buffer = (message as { d: ArrayBuffer }).d;
    expect([...new Uint8Array(buffer)]).toEqual([1, 2, 3]);
    expect(transfer).toEqual([buffer]);
    // Not the carrier's own bytes, which stay with the page.
    expect(buffer).not.toBe(whole.buffer);
    expect(pipe.consumedBytes).toBe(0);
    port.receive({ ack: 2 });
    port.receive({ ack: 1 });
    expect(pipe.consumedBytes).toBe(3);
  });

  it("writes what the view sends and ends its input when told", () => {
    const { pipe, port } = relay();
    port.receive({ w: new Uint8Array([104, 105]).buffer });
    port.receive({ fin: true });
    expect(pipe.written.map((bytes) => [...bytes])).toEqual([[104, 105]]);
    expect(pipe.finished).toBe(1);
    expect(pipe.closed).toBe(0);
  });

  it("tells the view the stream is over after its last piece, and lets go", () => {
    const { pipe, port, made, gone } = relay();
    pipe.receive(new Uint8Array([1]));
    pipe.ended(true);
    expect(port.messages().slice(-1)).toEqual([{ end: true, ok: true }]);
    expect(port.closed).toBe(true);
    expect(made.closed).toBe(true);
    expect(pipe.closed).toBe(1);
    expect(gone()).toBe(1);
  });

  it("ends a stream whose view breaks its rules", () => {
    for (const bad of [
      { ack: 5 },
      { ack: -1 },
      { ack: 0.5 },
      { w: new ArrayBuffer(WRITE_MAX + 1) },
      { w: "text" },
      { close: true },
      "ack",
      null,
    ]) {
      const { pipe, port, made } = relay();
      pipe.receive(new Uint8Array(4));
      port.receive(bad);
      expect(made.closed, JSON.stringify(bad)).toBe(true);
      expect(pipe.closed).toBe(1);
      // Nothing more reaches the view or the stream.
      const sent = port.sent.length;
      pipe.receive(new Uint8Array(4));
      port.receive({ w: new ArrayBuffer(1) });
      expect(port.sent.length).toBe(sent);
      expect(pipe.written).toEqual([]);
    }
  });

  it("ends a stream whose view floods its port", () => {
    const { pipe, port, made } = relay();
    for (let i = 0; i < 2000 && !made.closed; i += 1) port.receive({ ack: 0 });
    expect(made.closed).toBe(true);
    expect(pipe.closed).toBe(1);
  });

  it("refuses to write to a stream that is only read", () => {
    const { pipe, port, made } = relay();
    pipe.writable = false;
    port.receive({ w: new ArrayBuffer(1) });
    expect(made.closed).toBe(true);
    expect(pipe.written).toEqual([]);
  });

  it("hands over for a short while at a time, and no faster than a stream may go", () => {
    const clock = new FakeClock();
    const { pipe, port } = relay(clock);
    // Each look at the clock costs a millisecond, so two pieces fill one go.
    clock.cost = 1;
    for (let i = 0; i < 6; i += 1) pipe.receive(new Uint8Array(10));
    expect(port.sent.length).toBeLessThan(6);
    const first = port.sent.length;
    clock.cost = 0;
    clock.advance(clock.at + 1);
    expect(port.sent.length).toBe(6);
    expect(first).toBeGreaterThan(0);

    // More at once than a stream may carry at once: what is over waits its turn.
    const big = new Uint8Array(64 * 1024);
    const pieces = STREAM_BYTES_BURST / big.length + 8;
    for (let i = 0; i < pieces; i += 1) pipe.receive(big);
    const sentNow = port.sent.length - 6;
    expect(sentNow).toBeLessThanOrEqual(STREAM_BYTES_BURST / big.length + 1);
    expect(sentNow).toBeGreaterThan(0);
    clock.advance(clock.at + 1000);
    expect(port.sent.length - 6).toBe(pieces);
  });

  it("holds a view to the streams it may have open, and closes them with it", async () => {
    const pipes: FakePipe[] = [];
    const streams = new FrameStreams(() => {
      const pipe = new FakePipe();
      pipes.push(pipe);
      return Promise.resolve(pipe);
    });
    const ports = () => ({ mine: new FakePort(), theirs: {} as Transferable });
    for (let i = 0; i < STREAMS_PER_FRAME; i += 1) await streams.open(`g${i}`, ports);
    await expect(streams.open("one-more", ports)).rejects.toMatchObject({ code: "busy" });
    expect(streams.count).toBe(STREAMS_PER_FRAME);
    // One that ends makes room.
    pipes[0]!.ended(false);
    expect(streams.count).toBe(STREAMS_PER_FRAME - 1);
    streams.close();
    expect(streams.count).toBe(0);
    expect(pipes.every((pipe) => pipe.closed === 1)).toBe(true);
    await expect(streams.open("late", ports)).rejects.toThrow();
  });
});

/** A carrier that answers the commands a pipe needs. */
class PipeCarrier implements Carrier {
  deliver: (frames: Uint8Array) => void = () => undefined;
  readonly requests: Uint8Array[] = [];
  readonly commands: { name: string; args: Record<string, unknown> }[] = [];
  refuse = false;

  open(receive: (frames: Uint8Array) => void): Promise<CarrierOpened> {
    this.deliver = receive;
    return Promise.resolve({
      carrier: 7,
      limits: LIMITS,
      creditWindow: 1000,
      defaultBackend: "local",
    });
  }

  send(frames: Uint8Array): Promise<void> {
    this.requests.push(frames);
    return Promise.resolve();
  }

  command<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
    this.commands.push({ name, args });
    if (this.refuse) {
      return Promise.reject({ code: "not_found", message: "there is no such grant" });
    }
    return Promise.resolve({ readable: true, writable: true } as T);
  }

  /** The kind and payload length of each frame the page sent, oldest first. */
  sent(): { kind: number; stream: number; len: number }[] {
    const out: { kind: number; stream: number; len: number }[] = [];
    for (const bytes of this.requests) {
      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
      for (let at = 0; at < bytes.length;) {
        const len = view.getUint32(at + 5, true);
        out.push({ stream: view.getUint32(at, true), kind: view.getUint8(at + 4), len });
        at += HEADER_LEN + len;
      }
    }
    return out;
  }
}

describe("a pipe on the page's carrier", () => {
  it("opens a grant as a stream of its own and carries bytes both ways", async () => {
    const carrier = new PipeCarrier();
    const client = await TransportClient.open(carrier, { onError: () => undefined });
    const pipe = await client.pipe("v3", "abcd", 100);
    expect(carrier.commands).toEqual([
      { name: "view_stream_open", args: { carrier: 7, view: "v3", grant: "abcd", stream: 1 } },
    ]);
    expect(pipe.readable && pipe.writable).toBe(true);

    // What arrives before the pipe is started waits for it.
    carrier.deliver(rawFrame(1, Kind.Data, [1, 2, 3]));
    const got: number[][] = [];
    const ends: boolean[] = [];
    pipe.start(
      (bytes) => got.push([...bytes]),
      (ok) => ends.push(ok),
    );
    expect(got).toEqual([[1, 2, 3]]);
    pipe.consumed(60);
    pipe.write(new Uint8Array([7, 8]));
    pipe.finish();
    await settle();
    expect(carrier.sent()).toEqual([
      { stream: 1, kind: Kind.Credit, len: 4 },
      { stream: 1, kind: Kind.Credit, len: 4 },
      { stream: 1, kind: Kind.Input, len: 2 },
      { stream: 1, kind: Kind.Input, len: 0 },
    ]);

    // The stream's end is told once; nothing is sent for a stream that is over.
    carrier.deliver(rawFrame(1, Kind.Exit, [0, 0, 0, 0, 0]));
    expect(ends).toEqual([true]);
    const before = carrier.requests.length;
    pipe.write(new Uint8Array([1]));
    pipe.close();
    await settle();
    expect(carrier.requests.length).toBe(before);
  });

  it("lets go of a pipe that is closed, and of one that was refused", async () => {
    const carrier = new PipeCarrier();
    const client = await TransportClient.open(carrier, { onError: () => undefined });
    const pipe = await client.pipe("v1", "abcd", 100);
    pipe.start(
      () => undefined,
      () => undefined,
    );
    pipe.close();
    await settle();
    expect(carrier.sent().map((frame) => frame.kind)).toEqual([Kind.Credit, Kind.Detach]);

    carrier.refuse = true;
    await expect(client.pipe("v1", "nope", 100)).rejects.toMatchObject({ code: "not_found" });
    // Frames for the stream that never was are dropped.
    carrier.deliver(rawFrame(2, Kind.Data, [1]));
  });
});
