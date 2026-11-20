import { describe, expect, it } from "vitest";

import {
  CALLS_IN_FLIGHT_MAX,
  FrameBridge,
  type FrameHost,
  MESSAGE_MAX,
  pageUrl,
  pluginBase,
  type PortLike,
  Rate,
  RATE_BURST,
  readMessage,
} from "../src/plugins/frame/bridge.ts";
import { keepFrameReload, reloadPluginFrames } from "../src/plugins/frame/registry.ts";
import { toCommandError } from "../src/transport/types.ts";

class FakePort implements PortLike {
  sent: unknown[] = [];
  closed = false;
  onmessage: ((event: MessageEvent) => void) | null = null;
  postMessage(message: unknown): void {
    this.sent.push(message);
  }
  close(): void {
    this.closed = true;
  }
  receive(data: unknown): void {
    this.onmessage?.({ data } as MessageEvent);
  }
}

function host(overrides: Partial<FrameHost> = {}) {
  const seen = {
    ready: 0,
    states: [] as unknown[],
    keys: [] as unknown[],
    stopped: [] as string[],
  };
  const made: FrameHost = {
    call: async (name, args) => ({ name, args }),
    ready: () => void (seen.ready += 1),
    state: (state) => void seen.states.push(state),
    key: (key) => void seen.keys.push(key),
    stream: () => Promise.reject(new Error("no stream")),
    stop: (reason) => void seen.stopped.push(reason),
    ...overrides,
  };
  return { made, seen };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("what a frame may send", () => {
  it("reads each kind of message and nothing else", () => {
    expect(readMessage({ t: "ready" })).toEqual({ t: "ready" });
    expect(readMessage({ t: "call", id: 3, name: "list", args: { a: 1 } })).toEqual({
      t: "call",
      id: 3,
      name: "list",
      args: { a: 1 },
    });
    expect(readMessage({ t: "key", key: "Tab", code: "Tab", mods: ["ctrl"] })).not.toBeNull();
    expect(readMessage({ t: "state", state: { scroll: 4 } })).toEqual({
      t: "state",
      state: { scroll: 4 },
    });
    for (const bad of [
      null,
      "ready",
      [],
      { t: "invoke" },
      { t: "call", id: -1, name: "x" },
      { t: "call", id: 1.5, name: "x" },
      { t: "call", id: 1, name: "" },
      { t: "call", id: 1, name: "x".repeat(200) },
      { t: "key", key: "v", code: "KeyV", mods: ["super"] },
      { t: "state", state: "x".repeat(MESSAGE_MAX + 1) },
    ]) {
      expect(readMessage(bad)).toBeNull();
    }
    const circular: Record<string, unknown> = { t: "state" };
    circular.state = circular;
    expect(readMessage(circular)).toBeNull();
  });

  it("holds a frame to its rate and stops one that keeps flooding", () => {
    const rate = new Rate(200, RATE_BURST, 0);
    let ok = 0;
    for (let i = 0; i < RATE_BURST; i += 1) if (rate.take(0) === "ok") ok += 1;
    expect(ok).toBe(RATE_BURST);
    expect(rate.take(0)).toBe("drop");
    // A second later two hundred more are allowed.
    for (let i = 0; i < 200; i += 1) expect(rate.take(1000)).toBe("ok");
    let outcome = "ok";
    for (let i = 0; i < 1000 && outcome !== "stop"; i += 1) outcome = rate.take(1000);
    expect(outcome).toBe("stop");
  });
});

describe("a frame's bridge", () => {
  it("answers calls and passes on what the frame says", async () => {
    const port = new FakePort();
    const { made, seen } = host();
    const bridge = new FrameBridge(port, made, () => 0);
    port.receive({ t: "ready" });
    port.receive({ t: "call", id: 1, name: "echo", args: [1] });
    port.receive({ t: "state", state: { at: 2 } });
    await settle();
    expect(seen.ready).toBe(1);
    expect(seen.states).toEqual([{ at: 2 }]);
    expect(port.sent).toEqual([{ t: "result", id: 1, value: { name: "echo", args: [1] } }]);
    bridge.event("changed", { n: 1 });
    bridge.tokens({ "--lume-color-surface": "#000" }, "dark");
    expect(port.sent.slice(1)).toEqual([
      { t: "event", event: "changed", payload: { n: 1 } },
      { t: "tokens", tokens: { "--lume-color-surface": "#000" }, scheme: "dark" },
    ]);
  });

  it("passes on a failure with its kind", async () => {
    const port = new FakePort();
    const { made } = host({
      call: () => Promise.reject(Object.assign(new Error("no"), { code: "denied" })),
    });
    new FrameBridge(port, made, () => 0);
    port.receive({ t: "call", id: 9, name: "x" });
    await settle();
    expect(port.sent).toEqual([{ t: "error", id: 9, code: "denied", message: "no" }]);
  });

  it("passes on what a plugin's code said of a failure of its own kind", async () => {
    const port = new FakePort();
    const { made } = host({
      call: () =>
        Promise.reject(
          toCommandError({ code: "conflict", message: "two files", data: { files: ["a.rs"] } }),
        ),
    });
    new FrameBridge(port, made, () => 0);
    port.receive({ t: "call", id: 3, name: "merge" });
    await settle();
    expect(port.sent).toEqual([
      { t: "error", id: 3, code: "conflict", message: "two files", data: { files: ["a.rs"] } },
    ]);
  });

  it("refuses calls past the number that may wait", async () => {
    const port = new FakePort();
    const { made } = host({ call: () => new Promise(() => undefined) });
    new FrameBridge(port, made, () => 0);
    for (let id = 0; id <= CALLS_IN_FLIGHT_MAX; id += 1) port.receive({ t: "call", id, name: "x" });
    await settle();
    expect(port.sent).toEqual([
      expect.objectContaining({ t: "error", id: CALLS_IN_FLIGHT_MAX, code: "busy" }),
    ]);
  });

  it("stops a frame that sends what is not a message, and hears nothing after", () => {
    const port = new FakePort();
    const { made, seen } = host();
    const bridge = new FrameBridge(port, made, () => 0);
    port.receive({ t: "invoke", cmd: "carrier_send" });
    expect(seen.stopped).toHaveLength(1);
    expect(port.closed).toBe(true);
    expect(bridge.stopped).toBe(true);
    port.receive({ t: "ready" });
    expect(seen.ready).toBe(0);
  });

  it("stops a frame that floods its port", () => {
    const port = new FakePort();
    const { made, seen } = host();
    new FrameBridge(port, made, () => 0);
    for (let i = 0; i < 5000; i += 1) port.receive({ t: "state", state: i });
    expect(seen.stopped).toEqual(["it sent far more messages than a view may"]);
    expect(seen.states.length).toBe(RATE_BURST);
  });

  it("stops a frame that sends more than it may, however seldom", () => {
    const port = new FakePort();
    const { made, seen } = host();
    new FrameBridge(port, made, () => 0);
    // Each is a message a view may send; together they are more than it may at once.
    const filler = "x".repeat(900 * 1024);
    for (let id = 0; id < 4 && seen.stopped.length === 0; id += 1) {
      port.receive({ t: "call", id, name: "sink", args: filler });
    }
    expect(seen.stopped).toEqual(["it sent far more than a view may"]);
  });

  it("stops a frame that says it failed", () => {
    const port = new FakePort();
    const { made, seen } = host();
    new FrameBridge(port, made, () => 0);
    port.receive({ t: "error", message: "boom" });
    expect(seen.stopped).toEqual(["it failed: boom"]);
  });
});

describe("the frames of a plugin that is being worked on", () => {
  it("are loaded again by their plugin, for as long as each is there", () => {
    const loaded: string[] = [];
    const first = keepFrameReload("me.work", () => loaded.push("first"));
    const second = keepFrameReload("me.work", () => loaded.push("second"));
    const other = keepFrameReload("acme.tools", () => loaded.push("other"));
    reloadPluginFrames("me.work");
    expect(loaded).toEqual(["first", "second"]);
    first();
    reloadPluginFrames("me.work");
    expect(loaded).toEqual(["first", "second", "second"]);
    second();
    other();
    reloadPluginFrames("me.work");
    reloadPluginFrames("acme.tools");
    expect(loaded).toHaveLength(3);
  });
});

describe("where pages are served from", () => {
  it("follows the system and encodes each part of the path", () => {
    expect(pluginBase("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe(
      "http://lume-plugin.localhost",
    );
    expect(pluginBase("Mozilla/5.0 (Macintosh; Intel Mac OS X)")).toBe("lume-plugin://localhost");
    expect(pageUrl("http://lume-plugin.localhost", "acme.review", "ui/my page.html")).toBe(
      "http://lume-plugin.localhost/acme.review/ui/my%20page.html",
    );
  });
});
