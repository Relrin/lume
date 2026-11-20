import { describe, expect, it } from "vitest";

import { noticeKey, NoticeStore, type ShownNotice } from "../src/shell/notices.ts";
import { NOTICE_INFO_MS, NOTICE_PROBLEM_MS, NOTICES_MAX } from "../src/tuning.ts";
import { createWindowStateStore } from "../src/window/state.ts";

/** Something that shows notices, and what it was handed. */
function surface() {
  const shown: ShownNotice[] = [];
  const hidden: string[] = [];
  const messages = () => shown.map((notice) => notice.message);
  return {
    shown,
    hidden,
    messages,
    show: (notice: ShownNotice) => void shown.push(notice),
    hide: (key: string) => void hidden.push(key),
  };
}

describe("the window's notices", () => {
  it("hands a notice to what shows them, with how long it stays", () => {
    const store = new NoticeStore();
    const toaster = surface();
    store.connect(toaster);
    store.push("info", "saved");
    store.push("error", "could not save");
    expect(toaster.shown).toMatchObject([
      { level: "info", message: "saved", stays: NOTICE_INFO_MS },
      { level: "error", message: "could not save", stays: NOTICE_PROBLEM_MS },
    ]);
  });

  it("keeps what is said before anything can show it, and hands it over then", () => {
    const store = new NoticeStore();
    store.push("warning", "early");
    store.push("info", "also early");
    const toaster = surface();
    store.connect(toaster);
    expect(toaster.messages()).toEqual(["early", "also early"]);
    // Handed over once: what shows them next is not told of them again.
    const next = surface();
    store.connect(next);
    expect(next.shown).toEqual([]);
  });

  it("keeps only the newest of what waits, and one of what is said twice", () => {
    const store = new NoticeStore();
    for (let number = 0; number < NOTICES_MAX + 2; number++) store.push("error", `fault ${number}`);
    store.push("error", `fault ${NOTICES_MAX + 1}`);
    const toaster = surface();
    store.connect(toaster);
    expect(toaster.shown).toHaveLength(NOTICES_MAX);
    expect(toaster.messages()[0]).toBe("fault 2");
    expect(toaster.messages().at(-1)).toBe(`fault ${NOTICES_MAX + 1}`);
  });

  it("names a notice said again by the same key, so it takes the first one's place", () => {
    const store = new NoticeStore();
    const toaster = surface();
    store.connect(toaster);
    store.push("warning", "the pane is too small");
    store.push("warning", "the pane is too small");
    expect(toaster.shown).toHaveLength(2);
    expect(toaster.shown[0]!.key).toBe(toaster.shown[1]!.key);
  });

  it("tells the same words apart when they matter differently or offer something", () => {
    const undo = { label: "Undo", command: "lume.group.bring-back" };
    const keys = new Set([
      noticeKey("info", "reloaded"),
      noticeKey("error", "reloaded"),
      noticeKey("info", "reloaded", undo),
    ]);
    expect(keys.size).toBe(3);
  });

  it("takes a notice away by its key, shown or still waiting", () => {
    const store = new NoticeStore();
    store.push("info", "one");
    store.push("info", "two");
    store.dismiss(noticeKey("info", "one"));
    const toaster = surface();
    const stop = store.connect(toaster);
    expect(toaster.messages()).toEqual(["two"]);
    store.dismiss(noticeKey("info", "two"));
    expect(toaster.hidden).toEqual([noticeKey("info", "two")]);
    // What no longer shows them is told nothing more.
    stop();
    store.push("info", "three");
    expect(toaster.messages()).toEqual(["two"]);
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
