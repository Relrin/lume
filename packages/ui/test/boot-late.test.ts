import { describe, expect, it } from "vitest";

import { Disposer } from "../src/shell/boot/disposer.ts";
import { late } from "../src/shell/boot/late.ts";

describe("a cell for what the start makes later", () => {
  it("says what was read too early, by name", () => {
    const cell = late<number>("the views");
    expect(() => cell.get()).toThrow("the views used before boot reached it");
    expect(cell.peek()).toBeUndefined();
  });

  it("gives the thing once it is set", () => {
    const cell = late<{ readonly name: string }>("the commands");
    const commands = { name: "run" };
    cell.set(commands);
    expect(cell.get()).toBe(commands);
    expect(cell.peek()).toBe(commands);
  });

  it("is set once only", () => {
    const cell = late<number>("the search");
    cell.set(1);
    expect(() => cell.set(2)).toThrow("the search was set twice");
    expect(cell.get()).toBe(1);
  });

  it("keeps what waits for the thing until it is set, in order", () => {
    const cell = late<string>("the views");
    const used: string[] = [];
    cell.whenSet((views) => used.push(`first ${views}`));
    cell.whenSet((views) => used.push(`second ${views}`));
    expect(used).toEqual([]);
    cell.set("made");
    expect(used).toEqual(["first made", "second made"]);
    cell.whenSet((views) => used.push(`after ${views}`));
    expect(used.at(-1)).toBe("after made");
  });

  it("holds a falsy thing as a thing", () => {
    const cell = late<number>("a count");
    cell.set(0);
    expect(cell.get()).toBe(0);
  });
});

describe("what the page undoes when it ends", () => {
  it("undoes the last thing first", () => {
    const disposer = new Disposer();
    const undone: string[] = [];
    disposer.add(() => undone.push("host"));
    disposer.use({ dispose: () => undone.push("views") });
    disposer.add(() => undone.push("listeners"));
    disposer.dispose();
    expect(undone).toEqual(["listeners", "views", "host"]);
  });

  it("undoes once, however often it is asked", () => {
    const disposer = new Disposer();
    let undone = 0;
    disposer.add(() => undone++);
    disposer.dispose();
    disposer.dispose();
    expect(undone).toBe(1);
  });

  it("gives back what it was handed to dispose", () => {
    const disposer = new Disposer();
    const thing = { dispose: () => undefined };
    expect(disposer.use(thing)).toBe(thing);
  });

  it("undoes everything though a step throws, and throws the first error then", () => {
    const disposer = new Disposer();
    const undone: string[] = [];
    disposer.add(() => undone.push("first"));
    disposer.add(() => {
      throw new Error("second failed");
    });
    disposer.add(() => {
      throw new Error("third failed");
    });
    expect(() => disposer.dispose()).toThrow("third failed");
    expect(undone).toEqual(["first"]);
  });

  it("can be handed on as a function", () => {
    const disposer = new Disposer();
    let undone = false;
    disposer.add(() => (undone = true));
    const { dispose } = disposer;
    dispose();
    expect(undone).toBe(true);
  });
});
