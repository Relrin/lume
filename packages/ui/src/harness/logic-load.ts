/**
 * Loads the core's layout logic into the harness page, once. The module is built into
 * `packages/ui/logic/` by `cargo xtask logic`, which `cargo xtask build` runs.
 */

import logicUrl from "../../logic/lume-logic.wasm?url";

import { LayoutLogic } from "./logic.ts";

let loading: Promise<LayoutLogic> | undefined;

/** The core's layout logic, loaded the first time it is asked for. */
export function layoutLogic(): Promise<LayoutLogic> {
  loading ??= fetch(logicUrl)
    .then((response) => {
      if (!response.ok) throw new Error(`the layout logic is not there (${response.status})`);
      return response.arrayBuffer();
    })
    .then((bytes) => LayoutLogic.load(bytes));
  return loading;
}
