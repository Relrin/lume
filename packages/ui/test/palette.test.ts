import { describe, expect, it } from "vitest";

import { promptsFor, readArg, type ArgPrompt } from "../src/palette/args.ts";
import {
  createPaletteStore,
  readRecents,
  rememberRecent,
  type RecentStorage,
} from "../src/palette/store.ts";
import { PALETTE_RECENTS_MAX } from "../src/tuning.ts";

describe("what the palette asks for before running a command", () => {
  it("asks for nothing when a command takes nothing or requires nothing", () => {
    expect(promptsFor({})).toEqual([]);
    expect(promptsFor({ args: null })).toEqual([]);
    expect(
      promptsFor({
        args: { type: "object", properties: { pane: { type: "string" } }, required: [] },
      }),
    ).toEqual([]);
  });

  it("asks for each required argument, in the order they are required", () => {
    const prompts = promptsFor({
      args: {
        type: "object",
        properties: {
          root: { type: ["string", "null"], description: "A folder" },
          name: { type: "string", description: "The new workspace's name" },
          pane: { type: "string" },
        },
        required: ["name", "root"],
      },
    });
    expect(prompts).toEqual([
      {
        name: "name",
        kind: "string",
        placeholder: "The new workspace's name",
        choices: null,
        minimum: null,
      },
      { name: "root", kind: "string", placeholder: "A folder", choices: null, minimum: null },
    ]);
  });

  it("offers the workspaces' names for an argument that says so", () => {
    const [prompt] = promptsFor({
      args: {
        type: "object",
        properties: { name: { type: "string", "x-lume-choices": "workspaces" } },
        required: ["name"],
      },
    });
    expect(prompt).toMatchObject({ name: "name", choices: "workspaces", placeholder: "name" });
  });

  it("leaves out what cannot be asked for in a line of text", () => {
    const prompts = promptsFor({
      args: {
        type: "object",
        properties: {
          index: { type: "integer", minimum: 1 },
          flag: { type: "boolean" },
          list: { type: "array" },
        },
        required: ["flag", "index", "list", "absent", 7],
      },
    });
    expect(prompts.map((prompt) => prompt.name)).toEqual(["index"]);
    expect(prompts[0]).toMatchObject({ kind: "integer", minimum: 1 });
  });

  it("asks for nothing when the schema is not one it can read", () => {
    expect(promptsFor({ args: "name" })).toEqual([]);
    expect(promptsFor({ args: { required: ["name"] } })).toEqual([]);
    expect(promptsFor({ args: { required: "name", properties: {} } })).toEqual([]);
  });
});

describe("reading what was typed in answer to a prompt", () => {
  const word: ArgPrompt = {
    name: "name",
    kind: "string",
    placeholder: "",
    choices: null,
    minimum: null,
  };
  const number: ArgPrompt = { ...word, name: "index", kind: "integer", minimum: 1 };

  it("takes words as they are, without the space around them", () => {
    expect(readArg(word, "  my work ")).toEqual({ ok: true, value: "my work" });
  });

  it("refuses nothing at all", () => {
    expect(readArg(word, "   ").ok).toBe(false);
    expect(readArg(number, "").ok).toBe(false);
  });

  it("takes a whole number and refuses anything else", () => {
    expect(readArg(number, " 3 ")).toEqual({ ok: true, value: 3 });
    expect(readArg(number, "3.5").ok).toBe(false);
    expect(readArg(number, "three").ok).toBe(false);
    expect(readArg(number, "1e3").ok).toBe(false);
    expect(readArg(number, "99999999999999999999").ok).toBe(false);
  });

  it("holds a number to the least it may be", () => {
    expect(readArg(number, "0")).toEqual({ ok: false, problem: "The number must be at least 1." });
    expect(readArg({ ...number, minimum: null }, "-4")).toEqual({ ok: true, value: -4 });
  });
});

describe("whether the palette is on show", () => {
  it("is put away until it is opened, and each opening is a new one", () => {
    const store = createPaletteStore();
    expect(store.getState().shown).toBe(false);
    store.getState().open();
    expect(store.getState()).toMatchObject({ shown: true, command: null, opening: 1 });
    store.getState().openFor("lume.workspace.new");
    expect(store.getState()).toMatchObject({
      shown: true,
      command: "lume.workspace.new",
      opening: 2,
    });
    store.getState().close();
    expect(store.getState()).toMatchObject({ shown: false, command: null, opening: 2 });
  });

  it("is shown and put away in turn by the key that toggles it", () => {
    const store = createPaletteStore();
    store.getState().toggle();
    expect(store.getState().shown).toBe(true);
    store.getState().toggle();
    expect(store.getState().shown).toBe(false);
  });

  it("tells nobody when it is put away while it is put away", () => {
    const store = createPaletteStore();
    let told = 0;
    store.subscribe(() => (told += 1));
    store.getState().close();
    expect(told).toBe(0);
  });
});

/** A page's storage that holds what it is given, or refuses everything. */
function storage(broken = false): RecentStorage & { readonly kept: Map<string, string> } {
  const kept = new Map<string, string>();
  return {
    kept,
    getItem: (key) => {
      if (broken) throw new Error("storage is forbidden");
      return kept.get(key) ?? null;
    },
    setItem: (key, value) => {
      if (broken) throw new Error("storage is forbidden");
      kept.set(key, value);
    },
  };
}

describe("the commands run from the palette lately", () => {
  it("remembers them, most recent first, each once", () => {
    const page = storage();
    expect(readRecents(page)).toEqual([]);
    rememberRecent(page, "lume.tab.new");
    rememberRecent(page, "lume.pane.zoom");
    expect(rememberRecent(page, "lume.tab.new")).toEqual(["lume.tab.new", "lume.pane.zoom"]);
    expect(readRecents(page)).toEqual(["lume.tab.new", "lume.pane.zoom"]);
  });

  it("remembers no more than it is meant to", () => {
    const page = storage();
    for (let number = 0; number < PALETTE_RECENTS_MAX + 5; number++) {
      rememberRecent(page, `lume.test.command-${number}`);
    }
    const recents = readRecents(page);
    expect(recents).toHaveLength(PALETTE_RECENTS_MAX);
    expect(recents[0]).toBe(`lume.test.command-${PALETTE_RECENTS_MAX + 4}`);
  });

  it("remembers nothing when the page has no storage, or the storage holds something else", () => {
    expect(readRecents(undefined)).toEqual([]);
    expect(rememberRecent(undefined, "lume.tab.new")).toEqual(["lume.tab.new"]);
    expect(readRecents(storage(true))).toEqual([]);
    expect(rememberRecent(storage(true), "lume.tab.new")).toEqual(["lume.tab.new"]);

    const odd = storage();
    odd.kept.set("lume.palette.recent", "{not json");
    expect(readRecents(odd)).toEqual([]);
    odd.kept.set("lume.palette.recent", JSON.stringify({ a: 1 }));
    expect(readRecents(odd)).toEqual([]);
    odd.kept.set("lume.palette.recent", JSON.stringify(["lume.tab.new", 4, null]));
    expect(readRecents(odd)).toEqual(["lume.tab.new"]);
  });
});
