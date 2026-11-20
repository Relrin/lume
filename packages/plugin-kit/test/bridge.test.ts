import { describe, expect, it } from "vitest";

import {
  CALLS_IN_FLIGHT_MAX,
  createView,
  type Hello,
  HostError,
  isHello,
  keyToPass,
  type PortLike,
  STATE_MAX,
  type Theme,
  worthForwarding,
} from "../src/bridge.ts";
import { applyTokens } from "../src/tokens.ts";

/** The window's end of a view's port: what the view sent, and a way to answer. */
class Window implements PortLike {
  readonly sent: Record<string, unknown>[] = [];
  onmessage: ((event: { data: unknown }) => void) | null = null;

  postMessage(message: unknown): void {
    this.sent.push(message as Record<string, unknown>);
  }

  say(data: unknown): void {
    this.onmessage?.({ data });
  }

  last(): Record<string, unknown> {
    const last = this.sent.at(-1);
    if (!last) throw new Error("nothing was sent");
    return last;
  }
}

const HELLO: Hello = {
  lume: "connect",
  api: 1,
  instance: "main/v1",
  plugin: "acme.notes",
  view: "notes",
  place: "pane",
  state: { count: 2 },
  tokens: { "--lume-color-surface": "#101010" },
  scheme: "dark",
};

describe("the first message", () => {
  it("is known by what it carries", () => {
    expect(isHello(HELLO)).toBe(true);
    expect(isHello({ ...HELLO, lume: "other" })).toBe(false);
    expect(isHello({ ...HELLO, place: "window" })).toBe(false);
    expect(isHello(null)).toBe(false);
    expect(isHello("connect")).toBe(false);
  });

  it("makes a view that says it is ready, with the look set first", () => {
    const window = new Window();
    const looks: Theme[] = [];
    const view = createView(HELLO, window, { applyTheme: (theme) => looks.push(theme) });
    expect(window.sent).toEqual([{ t: "ready" }]);
    expect(looks).toEqual([{ tokens: HELLO.tokens, scheme: "dark" }]);
    expect(view.instance).toBe("main/v1");
    expect(view.plugin).toBe("acme.notes");
    expect(view.view).toBe("notes");
    expect(view.place).toBe("pane");
    expect(view.state.get()).toEqual({ count: 2 });
  });
});

describe("calls", () => {
  it("are answered by their numbers, in any order", async () => {
    const window = new Window();
    const view = createView(HELLO, window);
    const first = view.host.call<number>("add", { a: 1 });
    const second = view.host.call<string>("name");
    expect(window.sent.slice(1)).toEqual([
      { t: "call", id: 1, name: "add", args: { a: 1 } },
      { t: "call", id: 2, name: "name", args: null },
    ]);
    window.say({ t: "result", id: 2, value: "notes" });
    window.say({ t: "result", id: 1, value: 3 });
    await expect(first).resolves.toBe(3);
    await expect(second).resolves.toBe("notes");
  });

  it("fail with the kind of failure the application names", async () => {
    const window = new Window();
    const view = createView(HELLO, window);
    const call = view.host.call("save");
    window.say({ t: "error", id: 1, code: "denied", message: "not allowed" });
    const error = await call.catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(HostError);
    expect(error).toMatchObject({ code: "denied", message: "not allowed" });
  });

  it("fail with the plugin's own kind of failure and what its code said of it", async () => {
    const window = new Window();
    const view = createView(HELLO, window);
    const call = view.host.call("merge");
    window.say({
      t: "error",
      id: 1,
      code: "conflict",
      message: "two files conflict",
      data: { files: ["a.rs", "b.rs"] },
    });
    const error = await call.catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(HostError);
    expect(error).toMatchObject({ code: "conflict", data: { files: ["a.rs", "b.rs"] } });
    // One that says nothing more has no data.
    const plain = view.host.call("save");
    window.say({ t: "error", id: 2, code: "denied", message: "no" });
    expect(await plain.catch((caught: unknown) => caught)).toMatchObject({ data: null });
  });

  it("are refused here when they could not be sent", async () => {
    const window = new Window();
    const view = createView(HELLO, window);
    await expect(view.host.call("")).rejects.toMatchObject({ code: "invalid" });
    await expect(view.host.call("x".repeat(129))).rejects.toMatchObject({ code: "invalid" });
    await expect(view.host.call("big", "x".repeat(1024 * 1024))).rejects.toMatchObject({
      code: "invalid",
    });
    const loop: Record<string, unknown> = {};
    loop["self"] = loop;
    await expect(view.host.call("loop", loop)).rejects.toMatchObject({ code: "invalid" });
    expect(window.sent).toHaveLength(1);
  });

  it("wait for no more answers than the window lets a view wait for", async () => {
    const window = new Window();
    const view = createView(HELLO, window);
    for (let n = 0; n < CALLS_IN_FLIGHT_MAX; n++) void view.host.call("slow").catch(() => null);
    await expect(view.host.call("one more")).rejects.toMatchObject({ code: "busy" });
    window.say({ t: "result", id: 1, value: null });
    const after = view.host.call("after");
    window.say({ t: "result", id: CALLS_IN_FLIGHT_MAX + 1, value: "ok" });
    await expect(after).resolves.toBe("ok");
  });

  it("ignore an answer nobody waits for", () => {
    const window = new Window();
    createView(HELLO, window);
    window.say({ t: "result", id: 99, value: 1 });
    window.say({ t: "error", id: 99, code: "failed", message: "x" });
    window.say("noise");
    window.say({ t: "unknown" });
  });
});

describe("events", () => {
  it("reach every listener of their name until it stops listening", () => {
    const window = new Window();
    const view = createView(HELLO, window);
    const heard: unknown[] = [];
    const stop = view.host.on("changed", (payload) => heard.push(payload));
    view.host.on("changed", (payload) => heard.push(["second", payload]));
    view.host.on("other", () => heard.push("other"));
    window.say({ t: "event", event: "changed", payload: { n: 1 } });
    stop();
    window.say({ t: "event", event: "changed", payload: { n: 2 } });
    expect(heard).toEqual([{ n: 1 }, ["second", { n: 1 }], ["second", { n: 2 }]]);
  });
});

describe("what is kept", () => {
  it("is sent to the window and read back at once", () => {
    const window = new Window();
    const view = createView(HELLO, window);
    view.state.set({ count: 3 });
    expect(window.last()).toEqual({ t: "state", state: { count: 3 } });
    expect(view.state.get()).toEqual({ count: 3 });
  });

  it("is held to what the window keeps", () => {
    const window = new Window();
    const view = createView(HELLO, window);
    expect(() => view.state.set("x".repeat(STATE_MAX))).toThrow(RangeError);
    expect(() => view.state.set(() => 1)).toThrow(TypeError);
    expect(view.state.get()).toEqual({ count: 2 });
    expect(window.sent).toHaveLength(1);
    view.state.set("x".repeat(STATE_MAX - 2));
    expect(window.sent).toHaveLength(2);
  });

  it("tells whoever listens when the window refused something", () => {
    const window = new Window();
    const view = createView(HELLO, window);
    const refused: unknown[] = [];
    view.onRefused((refusal) => refused.push(refusal));
    window.say({ t: "refused", what: "state", message: "a view keeps 64 KiB at most" });
    expect(refused).toEqual([{ what: "state", message: "a view keeps 64 KiB at most" }]);
  });
});

describe("the look and the keyboard", () => {
  it("follows the window's look", () => {
    const window = new Window();
    const looks: Theme[] = [];
    const view = createView(HELLO, window, { applyTheme: (theme) => looks.push(theme) });
    const heard: Theme[] = [];
    view.theme.onChange((theme) => heard.push(theme));
    const before = view.theme.current();
    expect(view.theme.current()).toBe(before);
    window.say({ t: "tokens", tokens: { "--lume-color-surface": "#ffffff" }, scheme: "light" });
    const light = { tokens: { "--lume-color-surface": "#ffffff" }, scheme: "light" };
    expect(view.theme.current()).toEqual(light);
    expect(heard).toEqual([light]);
    expect(looks.at(-1)).toEqual(light);
  });

  it("sets only the window's own variables on the page", () => {
    const set: Record<string, string> = {};
    const attributes: Record<string, string> = {};
    applyTokens(
      { "--lume-color-text": "#eee", "--other": "1", color: "red", "--lume-BAD": "x" },
      "light",
      {
        style: { setProperty: (name, value) => void (set[name] = value) },
        setAttribute: (name, value) => void (attributes[name] = value),
      },
    );
    expect(set).toEqual({ "--lume-color-text": "#eee", "color-scheme": "light" });
    expect(attributes).toEqual({ "data-scheme": "light" });
  });

  it("takes the keyboard itself only when nobody listens for it", () => {
    const window = new Window();
    let taken = 0;
    const view = createView(HELLO, window, { focus: () => taken++ });
    window.say({ t: "focus" });
    expect(taken).toBe(1);
    let heard = 0;
    const stop = view.onFocus(() => heard++);
    window.say({ t: "focus" });
    expect([taken, heard]).toEqual([1, 1]);
    stop();
    window.say({ t: "focus" });
    expect(taken).toBe(2);
  });

  it("passes on a key and says when it failed", () => {
    const window = new Window();
    const view = createView(HELLO, window);
    view.passKey({ key: "p", code: "KeyP", mods: ["ctrl", "shift"] });
    expect(window.last()).toEqual({ t: "key", key: "p", code: "KeyP", mods: ["ctrl", "shift"] });
    view.fail("x".repeat(2000));
    expect(window.last()["t"]).toBe("error");
    expect(String(window.last()["message"])).toHaveLength(1000);
  });
});

describe("the keys worth passing on", () => {
  const key = (
    name: string,
    held: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {},
    more: { defaultPrevented?: boolean; isComposing?: boolean } = {},
  ) => ({
    key: name,
    code: `Key${name.toUpperCase()}`,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    ...held,
    ...more,
  });

  it("are those held with Ctrl, Alt or the logo key, and the function keys", () => {
    expect(worthForwarding({ key: "t", code: "KeyT", mods: ["ctrl", "shift"] })).toBe(true);
    expect(worthForwarding({ key: "ArrowLeft", code: "ArrowLeft", mods: ["alt"] })).toBe(true);
    expect(worthForwarding({ key: "k", code: "KeyK", mods: ["meta"] })).toBe(true);
    expect(worthForwarding({ key: "F5", code: "F5", mods: [] })).toBe(true);
    expect(worthForwarding({ key: "F24", code: "F24", mods: ["shift"] })).toBe(true);
    expect(worthForwarding({ key: "F25", code: "F25", mods: [] })).toBe(false);
    expect(worthForwarding({ key: "a", code: "KeyA", mods: [] })).toBe(false);
    expect(worthForwarding({ key: "A", code: "KeyA", mods: ["shift"] })).toBe(false);
    expect(worthForwarding({ key: "Control", code: "ControlLeft", mods: ["ctrl"] })).toBe(false);
  });

  it("leave out what the view used, and what a field uses itself", () => {
    expect(keyToPass(key("p", { ctrlKey: true, shiftKey: true }), false)).toEqual({
      key: "p",
      code: "KeyP",
      mods: ["ctrl", "shift"],
    });
    expect(keyToPass(key("p", { ctrlKey: true }, { defaultPrevented: true }), false)).toBeNull();
    expect(keyToPass(key("p", { ctrlKey: true }, { isComposing: true }), false)).toBeNull();
    expect(keyToPass(key("x"), false)).toBeNull();
    // Copying in a field stays in the field; outside one it is the window's to refuse.
    expect(keyToPass(key("c", { ctrlKey: true }), true)).toBeNull();
    expect(keyToPass(key("c", { ctrlKey: true }), false)).not.toBeNull();
    expect(keyToPass(key("ArrowLeft", { ctrlKey: true }), true)).toBeNull();
    expect(keyToPass(key("ArrowLeft", { altKey: true }), true)).not.toBeNull();
    expect(keyToPass(key("Tab", { ctrlKey: true }), true)).not.toBeNull();
  });
});
