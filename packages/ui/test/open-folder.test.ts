import { describe, expect, it } from "vitest";

import type { FolderList } from "../src/core/types.ts";
import {
  crumbsOf,
  cutPath,
  folderName,
  folderView,
  freeName,
  into,
  shownPath,
  upFrom,
} from "../src/shell/open-folder.ts";

const HOME = "C:\\Users\\kai";

function listing(
  path: string,
  names: readonly string[],
  extra: Partial<FolderList> = {},
): FolderList {
  return {
    home: HOME,
    separator: "\\",
    path,
    missing: false,
    more: false,
    branch: null,
    entries: names.map((name) => ({
      name,
      hidden: name.startsWith("."),
      branch: name === "lume" ? "main" : null,
    })),
    ...extra,
  };
}

const SRC = listing(`${HOME}\\src`, ["acme-api", "lume", "notes", "scratch", "web"]);

describe("a typed path", () => {
  it("is cut where the folder it is in ends", () => {
    expect(cutPath("")).toEqual({ dir: "~", partial: "" });
    expect(cutPath("~")).toEqual({ dir: "~", partial: "" });
    expect(cutPath("src")).toEqual({ dir: "~", partial: "src" });
    expect(cutPath("~\\src\\lu")).toEqual({ dir: "~\\src", partial: "lu" });
    expect(cutPath("~/src/")).toEqual({ dir: "~/src", partial: "" });
    expect(cutPath("C:\\")).toEqual({ dir: "C:\\", partial: "" });
    expect(cutPath("C:\\Us")).toEqual({ dir: "C:\\", partial: "Us" });
    expect(cutPath("/etc/ss")).toEqual({ dir: "/etc", partial: "ss" });
    expect(cutPath("/e")).toEqual({ dir: "/", partial: "e" });
  });

  it("goes one folder up, and no further than the home folder or a root", () => {
    expect(upFrom("~\\src\\lume\\")).toBe("~\\src\\");
    expect(upFrom("~\\src\\lu")).toBe("~\\");
    expect(upFrom("~\\src")).toBe("");
    expect(upFrom("C:\\Users\\")).toBe("C:\\");
    expect(upFrom("C:\\")).toBe("");
    expect(upFrom("/etc/ssh/")).toBe("/etc/");
  });

  it("shows the home folder as ~", () => {
    expect(shownPath(HOME, HOME, "\\")).toBe("~");
    expect(shownPath(`${HOME}\\src`, HOME, "\\")).toBe("~\\src");
    expect(shownPath("c:\\users\\KAI\\src", HOME, "\\")).toBe("~\\src");
    expect(shownPath("C:\\Windows", HOME, "\\")).toBe("C:\\Windows");
    expect(shownPath("/home/kaiser", "/home/kai", "/")).toBe("/home/kaiser");
    expect(into(`${HOME}\\src`, SRC)).toBe("~\\src\\");
  });
});

describe("the folders listed", () => {
  it("are those what is typed is found in, those it begins first, with the chosen one's rest", () => {
    const view = folderView(
      { typed: "~\\src\\e", chosen: 0, moved: false, hidden: false },
      listing(`${HOME}\\src`, ["acme-api", "edge", "lume", "web"]),
      [],
      "This PC",
    );
    const names = view.rows.flatMap((row) => (row.kind === "folder" ? [row.name] : []));
    expect(names).toEqual(["edge", "acme-api", "lume", "web"]);
    expect(view.ghost).toBe("dge\\");
    expect(view.target).toBe(`${HOME}\\src\\edge`);
  });

  it("open the folder typed out whole, and the one chosen once the arrows move", () => {
    const typed = folderView(
      { typed: "~\\src\\lume", chosen: 0, moved: false, hidden: false },
      SRC,
      [],
      "This PC",
    );
    expect([typed.target, typed.targetBranch]).toEqual([`${HOME}\\src\\lume`, "main"]);
    const into = folderView(
      { typed: "~\\src\\", chosen: 2, moved: false, hidden: false },
      SRC,
      [],
      "This PC",
    );
    // Nothing chosen with the arrows: the folder typed is the one opened.
    expect(into.target).toBe(`${HOME}\\src`);
    expect(into.showChoice).toBe(false);
    const moved = folderView(
      { typed: "~\\src\\", chosen: 2, moved: true, hidden: false },
      SRC,
      [],
      "This PC",
    );
    expect(moved.target).toBe(`${HOME}\\src\\notes`);
  });

  it("put the workspaces' folders first while nothing is typed, and hide hidden folders", () => {
    const view = folderView(
      { typed: "", chosen: 0, moved: false, hidden: false },
      listing(HOME, [".config", "src"]),
      [{ workspace: "lume", root: `${HOME}\\src\\lume` }],
      "This PC",
    );
    expect(view.rows.map((row) => (row.kind === "heading" ? row.label : row.name))).toEqual([
      "Recent on This PC",
      "lume",
      "~ · home",
      "src",
    ]);
    expect(view.target).toBe(`${HOME}\\src\\lume`);
    const shown = folderView(
      { typed: "", chosen: 0, moved: false, hidden: true },
      listing(HOME, [".config", "src"]),
      [],
      "This PC",
    );
    expect(shown.count).toBe(2);
  });

  it("say when the folder typed is not there", () => {
    const view = folderView(
      { typed: "~\\nowhere\\", chosen: 0, moved: false, hidden: false },
      listing(`${HOME}\\nowhere`, [], { missing: true }),
      [],
      "This PC",
    );
    expect([view.missing, view.target, view.count]).toEqual([true, null, 0]);
  });

  it("lead back through the folders above, each a place to go", () => {
    expect(crumbsOf(SRC)).toEqual([
      { label: "~", path: "~" },
      { label: "src", path: "~\\src" },
    ]);
    expect(crumbsOf(listing("C:\\Windows\\System32", []))).toEqual([
      { label: "C:", path: "C:\\" },
      { label: "Windows", path: "C:\\Windows" },
      { label: "System32", path: "C:\\Windows\\System32" },
    ]);
    expect(crumbsOf({ ...listing("/etc/ssh", []), home: "/home/kai", separator: "/" })).toEqual([
      { label: "/", path: "/" },
      { label: "etc", path: "/etc" },
      { label: "ssh", path: "/etc/ssh" },
    ]);
  });
});

describe("the workspace a folder becomes", () => {
  it("is named after the folder, and no other workspace's name", () => {
    expect(folderName(`${HOME}\\src\\lume`)).toBe("lume");
    expect(folderName("C:\\")).toBe("C");
    expect(folderName("~")).toBe("home");
    expect(freeName("lume", ["default"])).toBe("lume");
    expect(freeName("lume", ["lume", "lume-2"])).toBe("lume-3");
  });
});
