import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { NoticeStore } from "../src/shell/notices.ts";
import { NOTICE_INFO_MS, NOTICE_PROBLEM_MS, NOTICES_MAX } from "../src/tuning.ts";
import { createWindowStateStore } from "../src/window/state.ts";

describe("the window's notices", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  const messages = (store: NoticeStore) => store.current().map((notice) => notice.message);

  it("shows a notice and tells whoever listens", () => {
    const store = new NoticeStore();
    let told = 0;
    const stop = store.subscribe(() => (told += 1));
    store.push("info", "saved");
    expect(store.current()).toMatchObject([{ level: "info", message: "saved" }]);
    expect(told).toBe(1);
    stop();
    store.push("info", "again");
    expect(told).toBe(1);
  });

  it("takes a notice away after a while, a problem later than a piece of news", () => {
    const store = new NoticeStore();
    store.push("info", "saved");
    store.push("error", "could not save");
    vi.advanceTimersByTime(NOTICE_INFO_MS - 1);
    expect(messages(store)).toEqual(["saved", "could not save"]);
    vi.advanceTimersByTime(1);
    expect(messages(store)).toEqual(["could not save"]);
    vi.advanceTimersByTime(NOTICE_PROBLEM_MS - NOTICE_INFO_MS);
    expect(messages(store)).toEqual([]);
  });

  it("renews a notice that is said again instead of showing it twice", () => {
    const store = new NoticeStore();
    store.push("warning", "the pane is too small");
    const [first] = store.current();
    vi.advanceTimersByTime(NOTICE_PROBLEM_MS - 100);
    store.push("warning", "the pane is too small");
    expect(store.current()).toEqual([first]);
    vi.advanceTimersByTime(NOTICE_PROBLEM_MS - 100);
    expect(messages(store)).toEqual(["the pane is too small"]);
    vi.advanceTimersByTime(100);
    expect(messages(store)).toEqual([]);
  });

  it("tells the same words apart when they matter differently", () => {
    const store = new NoticeStore();
    store.push("info", "reloaded");
    store.push("error", "reloaded");
    expect(store.current()).toHaveLength(2);
  });

  it("makes room for a new notice by dropping the oldest", () => {
    const store = new NoticeStore();
    for (let number = 0; number < NOTICES_MAX + 2; number++) store.push("error", `fault ${number}`);
    expect(store.current()).toHaveLength(NOTICES_MAX);
    expect(messages(store)[0]).toBe("fault 2");
    expect(messages(store).at(-1)).toBe(`fault ${NOTICES_MAX + 1}`);
    // The ones that were dropped leave no timer behind to take another away.
    vi.advanceTimersByTime(NOTICE_PROBLEM_MS);
    expect(messages(store)).toEqual([]);
  });

  it("takes a notice away when it is dismissed, and nothing when it is not there", () => {
    const store = new NoticeStore();
    store.push("info", "one");
    store.push("info", "two");
    const [one] = store.current();
    let told = 0;
    store.subscribe(() => (told += 1));
    store.dismiss(one!.id);
    expect(messages(store)).toEqual(["two"]);
    store.dismiss(one!.id);
    store.dismiss(999);
    expect(told).toBe(1);
  });

  it("holds one and the same empty list while it shows nothing", () => {
    const store = new NoticeStore();
    const empty = store.current();
    store.push("info", "one");
    store.dismiss(store.current()[0]!.id);
    expect(store.current()).toBe(empty);
  });
});

describe("what a page holds of its window's state", () => {
  it("begins as an ordinary window in front", () => {
    expect(createWindowStateStore().getState()).toMatchObject({
      maximised: false,
      fullscreen: false,
      focused: true,
    });
  });

  it("takes what the core says and tells whoever listens only of a change", () => {
    const store = createWindowStateStore();
    let told = 0;
    store.subscribe(() => (told += 1));
    store.getState().set({ maximised: true, fullscreen: false, focused: true });
    expect(store.getState().maximised).toBe(true);
    store.getState().set({ maximised: true, fullscreen: false, focused: true });
    expect(told).toBe(1);
    store.getState().set({ maximised: true, fullscreen: false, focused: false });
    expect(told).toBe(2);
  });
});
