import { describe, expect, it } from "vitest";

import { createView, type Hello, type PortLike } from "../src/bridge.ts";
import { ACK_EVERY_MS, type StreamPortLike, ViewStream, WRITE_MAX } from "../src/streams.ts";

/** The application's end of a stream's port. */
class Far implements StreamPortLike {
  readonly sent: Record<string, unknown>[] = [];
  onmessage: ((event: { data: unknown }) => void) | null = null;
  closed = false;

  postMessage(message: unknown): void {
    this.sent.push(message as Record<string, unknown>);
  }

  close(): void {
    this.closed = true;
  }

  piece(bytes: number[]): void {
    this.onmessage?.({ data: { d: new Uint8Array(bytes).buffer } });
  }

  end(ok: boolean): void {
    this.onmessage?.({ data: { end: true, ok } });
  }

  acked(): number[] {
    return this.sent.filter((message) => "ack" in message).map((message) => Number(message["ack"]));
  }
}

/** A clock that is moved by hand. */
class Clock {
  at = 0;
  readonly tasks: { due: number; task: () => void }[] = [];

  now = () => this.at;

  later = (task: () => void, ms: number) => void this.tasks.push({ due: this.at + ms, task });

  pass(ms: number): void {
    this.at += ms;
    const due = this.tasks.filter((one) => one.due <= this.at);
    for (const one of due) {
      this.tasks.splice(this.tasks.indexOf(one), 1);
      one.task();
    }
  }
}

const BOTH = { readable: true, writable: true };

describe("a stream's pieces", () => {
  it("are handed on and said to be taken, which lets more come", () => {
    const far = new Far();
    const clock = new Clock();
    const stream = new ViewStream(far, BOTH, clock);
    const got: number[][] = [];
    stream.onData((piece) => void got.push([...piece]));
    far.piece([1, 2, 3]);
    expect(got).toEqual([[1, 2, 3]]);
    expect(far.acked()).toEqual([3]);
    expect(stream.bytes).toBe(3);
  });

  it("are said to be taken together when they come close together", () => {
    const far = new Far();
    const clock = new Clock();
    const stream = new ViewStream(far, BOTH, clock);
    stream.onData(() => undefined);
    far.piece([1]);
    far.piece([2, 3]);
    far.piece([4, 5, 6]);
    expect(far.acked()).toEqual([1]);
    clock.pass(ACK_EVERY_MS);
    expect(far.acked()).toEqual([1, 5]);
    clock.pass(ACK_EVERY_MS);
    far.piece([7]);
    expect(far.acked()).toEqual([1, 5, 1]);
  });

  it("wait, unacknowledged, until someone listens", () => {
    const far = new Far();
    const stream = new ViewStream(far, BOTH, new Clock());
    far.piece([1, 2]);
    far.piece([3]);
    expect(far.acked()).toEqual([]);
    const got: number[] = [];
    stream.onData((piece) => void got.push(...piece));
    expect(got).toEqual([1, 2, 3]);
    expect(far.acked()).toEqual([2]);
  });

  it("count as taken only when a listener that takes its time is done", async () => {
    const far = new Far();
    const stream = new ViewStream(far, BOTH, new Clock());
    let done: () => void = () => undefined;
    stream.onData(() => new Promise<void>((resolve) => (done = resolve)));
    far.piece([1, 2, 3, 4]);
    expect(far.acked()).toEqual([]);
    done();
    await Promise.resolve();
    await Promise.resolve();
    expect(far.acked()).toEqual([4]);
  });

  it("end with how the stream ended and how much came", async () => {
    const far = new Far();
    const stream = new ViewStream(far, BOTH, new Clock());
    stream.onData(() => undefined);
    far.piece([1, 2]);
    far.end(true);
    await expect(stream.ended).resolves.toEqual({ ok: true, bytes: 2 });
    expect(far.closed).toBe(true);
    expect(stream.closed).toBe(true);
  });

  it("end only after pieces that were waiting have been handed on", async () => {
    const far = new Far();
    const stream = new ViewStream(far, BOTH, new Clock());
    far.piece([9]);
    far.end(false);
    expect(stream.closed).toBe(false);
    const got: number[] = [];
    stream.onData((piece) => void got.push(...piece));
    await expect(stream.ended).resolves.toEqual({ ok: false, bytes: 1 });
    expect(got).toEqual([9]);
  });
});

describe("writing to a stream", () => {
  it("sends text as bytes, a long write in parts", () => {
    const far = new Far();
    const stream = new ViewStream(far, BOTH, new Clock());
    stream.write("hé");
    expect([...new Uint8Array(far.sent[0]?.["w"] as ArrayBuffer)]).toEqual([104, 195, 169]);
    stream.write(new Uint8Array(WRITE_MAX * 2 + 5));
    const sizes = far.sent.slice(1).map((message) => (message["w"] as ArrayBuffer).byteLength);
    expect(sizes).toEqual([WRITE_MAX, WRITE_MAX, 5]);
    stream.finish();
    expect(far.sent.at(-1)).toEqual({ fin: true });
  });

  it("is refused on a stream that is only read, and after it closed", async () => {
    const far = new Far();
    const stream = new ViewStream(far, { readable: true, writable: false }, new Clock());
    expect(() => stream.write("x")).toThrow("cannot be written");
    stream.finish();
    expect(far.sent).toEqual([]);
    stream.close();
    expect(far.sent).toEqual([{ close: true }]);
    expect(far.closed).toBe(true);
    await expect(stream.ended).resolves.toEqual({ ok: false, bytes: 0 });
    expect(() => stream.write("x")).toThrow("closed");
  });
});

describe("opening a stream", () => {
  const HELLO: Hello = {
    lume: "connect",
    api: 1,
    instance: "main/v1",
    plugin: "acme.notes",
    view: "notes",
    place: "panel",
    state: null,
    tokens: {},
    scheme: "dark",
  };

  it("asks the window with the grant and is given the stream's own port", async () => {
    const sent: Record<string, unknown>[] = [];
    const port: PortLike = {
      onmessage: null,
      postMessage: (message) => void sent.push(message as Record<string, unknown>),
    };
    const view = createView(HELLO, port);
    const opening = view.streams.open("0123456789abcdef");
    expect(sent.at(-1)).toEqual({ t: "stream", id: 1, grant: "0123456789abcdef" });
    const far = new Far();
    port.onmessage?.({ data: { t: "opened", id: 1, port: far, readable: true, writable: false } });
    const stream = await opening;
    expect([stream.readable, stream.writable]).toEqual([true, false]);
    far.piece([1]);
    expect(stream.bytes).toBe(1);
  });

  it("fails as the window says, and without asking for a grant that cannot be one", async () => {
    const sent: unknown[] = [];
    const port: PortLike = { onmessage: null, postMessage: (message) => void sent.push(message) };
    const view = createView(HELLO, port);
    await expect(view.streams.open("")).rejects.toMatchObject({ code: "not_found" });
    await expect(view.streams.open("g".repeat(65))).rejects.toMatchObject({ code: "not_found" });
    expect(sent).toHaveLength(1);
    const opening = view.streams.open("gone");
    port.onmessage?.({ data: { t: "error", id: 1, code: "not_found", message: "no such grant" } });
    await expect(opening).rejects.toMatchObject({ code: "not_found" });
  });
});
