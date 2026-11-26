/**
 * What the plugins add to the window. It comes as the core has it; nothing waits for
 * it. The core numbers each version, and tells the page of a newer one: the newest it
 * has told of is fetched, one at a time, and an older answer never replaces a newer.
 */

import type { CoreClient } from "../../core/client.ts";
import type { PluginsStore } from "../../plugins/store.ts";
import type { NoticeStore } from "../notices.ts";
import { reason } from "./reason.ts";

export interface ContributionsDeps {
  readonly client: Pick<CoreClient, "uiContributions">;
  readonly plugins: PluginsStore;
  readonly notices: Pick<NoticeStore, "push">;
  /** What the plugins add was taken in. */
  readonly changed: () => void;
}

export class Contributions {
  readonly #deps: ContributionsDeps;
  /** The revision held. */
  #revision = -1;
  /**
   * The newest revision the core has told of; one told of while a fetch is on its way
   * is fetched once that one is in.
   */
  #announced = -1;
  #fetching = false;

  constructor(deps: ContributionsDeps) {
    this.#deps = deps;
  }

  /** Fetches what the plugins add, unless a fetch is on its way. */
  fetch(): void {
    if (this.#fetching) return;
    this.#fetching = true;
    const { client, plugins, notices, changed } = this.#deps;
    client.uiContributions().then(
      (shown) => {
        this.#fetching = false;
        if (shown.revision >= this.#revision) {
          this.#revision = shown.revision;
          plugins.getState().set(shown);
          changed();
        }
        if (this.#announced > this.#revision) this.fetch();
      },
      (error: unknown) => {
        this.#fetching = false;
        notices.push("error", `What plugins add could not be read: ${reason(error)}`);
      },
    );
  }

  /** The core told of `revision`: it is fetched if it is newer than the one held. */
  announced(revision: number): void {
    this.#announced = Math.max(this.#announced, revision);
    if (revision > this.#revision) this.fetch();
  }
}
