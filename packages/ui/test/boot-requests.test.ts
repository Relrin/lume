import { describe, expect, it } from "vitest";

import { remoteTarget, type RequestDeps } from "../src/shell/boot/requests.ts";

describe("what the core asks of a window", () => {
  it("carries out a command once the views have followed the layout", async () => {
    // A command held while the page started comes as the page goes live, a moment
    // before the views would follow the layout's copy of their own accord.
    const done: string[] = [];
    let followed = false;
    const deps = {
      views: {
        reconcile: () => {
          followed = true;
          done.push("followed");
        },
      },
      commands: {
        runInPage: (id: string) => {
          if (!followed) throw new Error("no pane has the keyboard");
          done.push(id);
        },
      },
    } as unknown as RequestDeps;

    await remoteTarget(deps).command("lume.search.open", { query: "x" });

    expect(done).toEqual(["followed", "lume.search.open"]);
  });
});
