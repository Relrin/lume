import { describe, expect, it } from "vitest";

import type { Carrier, CarrierOpened } from "../src/transport/carrier.ts";
import { type StreamFrame, TransportClient } from "../src/transport/client.ts";
import { HEADER_LEN, Kind } from "../src/transport/frames.ts";
import type { ClientFacts } from "../src/transport/types.ts";
import { concat, LIMITS, rawFrame } from "./support.ts";

const WINDOW = 1000;

const BLACK = { r: 0, g: 0, b: 0 };
const FACTS: ClientFacts = {
  size: { cols: 80, rows: 24 },
  cellPx: { w: 8, h: 16 },
  palette: { fg: BLACK, bg: BLACK, cursor: BLACK, ansi: Array.from({ length: 16 }, () => BLACK) },
  scheme: "dark",
  focused: true,
  visible: true,
};

const ATTACHED = {
  size: { cols: 80, rows: 24 },
  resize_pulls_scrollback: false,
  hydrate: "none",
  caps: {
    persistent: false,
    multi_client: false,
    server_layout: false,
    responder: "client",
    hydrate: { snapshot: false, vt_replay: false },
  },
};

interface Sent {
  kind: number;
  stream: number;
  payload: Uint8Array;
}

/** Cuts what the page sent into its frames. */
function framesOf(bytes: Uint8Array): Sent[] {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
  const out: Sent[] = [];
  for (let at = 0; at < bytes.length;) {
    const len = view.getUint32(at + 5, true);
    out.push({
      stream: view.getUint32(at, true),
      kind: view.getUint8(at + 4),
      payload: bytes.slice(at + HEADER_LEN, at + HEADER_LEN + len),
    });
    at += HEADER_LEN + len;
  }
  return out;
}

/** A carrier with a test at the other end instead of the core. */
class FakeCarrier implements Carrier {
  /** Delivers frames to the page, as the core would. */
  deliver: (frames: Uint8Array) => void = () => undefined;
  /** Each request the page has made, oldest first. */
  readonly requests: Uint8Array[] = [];
  readonly commands: { name: string; args: Record<string, unknown> }[] = [];
  /** How many sends are waiting for an answer right now, and the most there ever were. */
  inFlight = 0;
  mostInFlight = 0;
  /** When set, sends wait until {@link answer} is called. */
  hold = false;
  /** When set, the next attach waits until this is called. */
  attachGate: Promise<void> | undefined;
  /** When set, sends are refused with this. */
  refusal: unknown;
  #waiting: (() => void)[] = [];

  open(receive: (frames: Uint8Array) => void): Promise<CarrierOpened> {
    this.deliver = receive;
    return Promise.resolve({
      carrier: 7,
      limits: LIMITS,
      creditWindow: WINDOW,
      defaultBackend: "local",
    });
  }

  async send(frames: Uint8Array): Promise<void> {
    this.requests.push(frames);
    this.inFlight++;
    this.mostInFlight = Math.max(this.mostInFlight, this.inFlight);
    if (this.hold) await new Promise<void>((resolve) => this.#waiting.push(resolve));
    this.inFlight--;
    if (this.refusal !== undefined) throw this.refusal;
  }

  /** Answers the oldest held send. */
  answer(): void {
    this.#waiting.shift()?.();
  }

  async command<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
    this.commands.push({ name, args });
    if (name === "session_attach") {
      await this.attachGate;
      return ATTACHED as T;
    }
    if (name === "session_spawn") return "0123456789abcdef" as T;
    if (name === "session_list") return [] as T;
    return undefined as T;
  }

  /** Every frame the page has sent, in the order the requests were made. */
  sent(): Sent[] {
    return this.requests.flatMap(framesOf);
  }
}

/** Lets queued microtasks and settled promises run. */
async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

async function opened(): Promise<{
  carrier: FakeCarrier;
  client: TransportClient;
  failures: Error[];
}> {
  const carrier = new FakeCarrier();
  const failures: Error[] = [];
  const client = await TransportClient.open(carrier, {
    onFailure: (error) => failures.push(error),
    onError: () => undefined,
  });
  return { carrier, client, failures };
}

const text = (value: string) => new TextEncoder().encode(value);
const data = (stream: number, payload: number[] | Uint8Array) =>
  rawFrame(stream, Kind.Data, payload);
const EXIT = rawFrame(1, Kind.Exit, [0, 0, 0, 0, 0]);

describe("the transport client", () => {
  it("names its carrier in every command", async () => {
    const { carrier, client } = await opened();
    expect(client.defaultBackend).toBe("local");
    await client.spawn("local", { size: { cols: 80, rows: 24 } });
    await client.list();
    await client.kill("local", "0123456789abcdef");
    expect(carrier.commands.map((call) => call.name)).toEqual([
      "session_spawn",
      "session_list",
      "session_kill",
    ]);
    for (const call of carrier.commands) expect(call.args.carrier).toBe(7);
    expect(carrier.commands[0]!.args.spec).toEqual({
      size: { cols: 80, rows: 24 },
      launcher: null,
      cwd: null,
      env: [],
    });
  });

  it("keeps frames that arrive before the view is ready and hands them over in order", async () => {
    const { carrier, client } = await opened();
    let open!: () => void;
    carrier.attachGate = new Promise((resolve) => (open = resolve));

    const attaching = client.attach("local", "0123456789abcdef", FACTS);
    await settle();
    // The core may send on the stream before it has answered the attach.
    carrier.deliver(rawFrame(1, Kind.Resized, [100, 0, 30, 0]));
    open();
    const attachment = await attaching;
    carrier.deliver(EXIT);

    const got: StreamFrame[] = [];
    attachment.start((frame) => got.push(frame));
    expect(got.map((frame) => frame.kind)).toEqual(["resized", "exit"]);
    expect(attachment.size).toEqual({ cols: 80, rows: 24 });
    expect(attachment.caps.responder).toBe("client");
  });

  it("grants the first window only when the view starts", async () => {
    const { carrier, client } = await opened();
    const attachment = await client.attach("local", "0123456789abcdef", FACTS);
    await settle();
    expect(carrier.sent()).toEqual([]);

    attachment.start(() => undefined);
    await settle();
    const sent = carrier.sent();
    expect(sent).toHaveLength(1);
    expect(sent[0]!.kind).toBe(Kind.Credit);
    expect(new DataView(sent[0]!.payload.buffer).getUint32(0, true)).toBe(WINDOW);
  });

  it("grants more once half a window has been consumed", async () => {
    const { carrier, client } = await opened();
    const attachment = await client.attach("local", "0123456789abcdef", FACTS);
    attachment.start((frame) => {
      if (frame.kind === "data") attachment.consumed(frame.bytes.length);
    });
    await settle();
    carrier.requests.length = 0;

    carrier.deliver(data(1, new Uint8Array(WINDOW / 2 - 1)));
    await settle();
    expect(carrier.sent()).toEqual([]);
    carrier.deliver(data(1, [1]));
    await settle();
    const sent = carrier.sent();
    expect(sent.map((frame) => frame.kind)).toEqual([Kind.Credit]);
    expect(new DataView(sent[0]!.payload.buffer).getUint32(0, true)).toBe(WINDOW / 2);
  });

  it("sends everything queued in one turn as one request", async () => {
    const { carrier, client } = await opened();
    const attachment = await client.attach("local", "0123456789abcdef", FACTS);
    attachment.start(() => undefined);
    attachment.input(text("a"));
    attachment.resize({ cols: 100, rows: 30 }, { w: 8, h: 16 });
    attachment.facts(FACTS);
    await settle();
    expect(carrier.requests).toHaveLength(1);
    expect(carrier.sent().map((frame) => frame.kind)).toEqual([
      Kind.Credit,
      Kind.Input,
      Kind.Resize,
      Kind.Facts,
    ]);
  });

  it("never has two requests in flight and keeps input in order", async () => {
    const { carrier, client } = await opened();
    const attachment = await client.attach("local", "0123456789abcdef", FACTS);
    attachment.start(() => undefined);
    await settle();
    carrier.requests.length = 0;
    carrier.hold = true;

    attachment.input(text("a"));
    await settle();
    attachment.input(text("b"));
    await settle();
    attachment.input(text("c"));
    await settle();
    // The first is unanswered, so the others wait rather than race it.
    expect(carrier.requests).toHaveLength(1);

    carrier.answer();
    await settle();
    expect(carrier.requests).toHaveLength(2);
    carrier.answer();
    await settle();

    expect(carrier.mostInFlight).toBe(1);
    const typed = concat(...carrier.sent().map((frame) => frame.payload));
    expect(new TextDecoder().decode(typed)).toBe("abc");
  });

  it("splits input longer than one payload", async () => {
    const { carrier, client } = await opened();
    const attachment = await client.attach("local", "0123456789abcdef", FACTS);
    attachment.start(() => undefined);
    await settle();
    carrier.requests.length = 0;

    attachment.input(new Uint8Array(LIMITS.maxPayload + 10));
    await settle();
    expect(carrier.sent().map((frame) => frame.payload.length)).toEqual([LIMITS.maxPayload, 10]);
  });

  it("gives each attachment a stream of its own and never uses one twice", async () => {
    const { carrier, client } = await opened();
    const first = await client.attach("local", "0123456789abcdef", FACTS);
    first.detach();
    await client.attach("local", "0123456789abcdef", FACTS);
    const streams = carrier.commands
      .filter((call) => call.name === "session_attach")
      .map((call) => call.args.stream);
    expect(streams).toEqual([1, 2]);
  });

  it("stops delivering to an attachment once it is detached", async () => {
    const { carrier, client } = await opened();
    const attachment = await client.attach("local", "0123456789abcdef", FACTS);
    const got: StreamFrame[] = [];
    attachment.start((frame) => got.push(frame));
    await settle();
    carrier.requests.length = 0;

    attachment.detach();
    attachment.input(text("late"));
    carrier.deliver(data(1, [1]));
    await settle();

    expect(got).toEqual([]);
    expect(carrier.sent().map((frame) => frame.kind)).toEqual([Kind.Detach]);
  });

  it("drops frames for a stream it does not have", async () => {
    const { carrier, failures } = await opened();
    carrier.deliver(data(42, [1, 2, 3]));
    expect(failures).toEqual([]);
  });

  it("keeps the other views going when one throws", async () => {
    const { carrier, client, failures } = await opened();
    const first = await client.attach("local", "0123456789abcdef", FACTS);
    const second = await client.attach("local", "0123456789abcdef", FACTS);
    first.start(() => {
      throw new Error("a view's fault");
    });
    const got: StreamFrame[] = [];
    second.start((frame) => got.push(frame));

    carrier.deliver(concat(data(1, [1]), data(2, [2])));
    expect(got).toHaveLength(1);
    expect(failures).toEqual([]);
  });

  it("fails for good when the core sends more than it was granted", async () => {
    const { carrier, client, failures } = await opened();
    const attachment = await client.attach("local", "0123456789abcdef", FACTS);
    const got: StreamFrame[] = [];
    attachment.start((frame) => got.push(frame));

    carrier.deliver(data(1, new Uint8Array(WINDOW + 1)));
    expect(failures).toHaveLength(1);
    expect(got).toEqual([]);
    carrier.deliver(data(1, [1]));
    expect(got).toEqual([]);
    await expect(client.list()).rejects.toBe(failures[0]);
  });

  it("fails for good when the bytes are not frames or carry a control message", async () => {
    const bad = await opened();
    bad.carrier.deliver(rawFrame(1, 0x7f, []));
    expect(bad.failures).toHaveLength(1);

    const control = await opened();
    control.carrier.deliver(rawFrame(0, Kind.Control, text("{}")));
    expect(control.failures).toHaveLength(1);
  });

  it("fails for good when the core no longer knows its carrier", async () => {
    const { carrier, client, failures } = await opened();
    const attachment = await client.attach("local", "0123456789abcdef", FACTS);
    carrier.refusal = { code: "stale_carrier", message: "the carrier has been replaced" };
    attachment.start(() => undefined);
    await settle();
    expect(failures.map((error) => error.message)).toEqual(["the carrier has been replaced"]);
  });

  it("carries on after a request the core refused for another reason", async () => {
    const { carrier, client, failures } = await opened();
    const attachment = await client.attach("local", "0123456789abcdef", FACTS);
    carrier.refusal = { code: "bad_request", message: "no" };
    attachment.start(() => undefined);
    await settle();
    carrier.refusal = undefined;
    attachment.input(text("x"));
    await settle();
    expect(failures).toEqual([]);
    expect(carrier.requests).toHaveLength(2);
  });

  it("forgets a stream whose attach was refused", async () => {
    const carrier = new FakeCarrier();
    const client = await TransportClient.open(carrier);
    carrier.command = () => Promise.reject({ code: "not_found", message: "no such session" });
    await expect(client.attach("local", "0123456789abcdef", FACTS)).rejects.toMatchObject({
      code: "not_found",
    });
    // Nothing is listening on the stream that was claimed for it.
    carrier.deliver(data(1, [1]));
  });
});
