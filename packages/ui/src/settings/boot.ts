/**
 * Starting the settings window's page: once for each page, however often a component
 * mounts.
 *
 * The window owns nothing that lasts. The configuration is the core's: the page asks
 * for it once, with the schema that says how to show it, and follows what it is told
 * from then on. A setting is changed by asking the core to write it; the change comes
 * back like any other, so what the window shows is always what is in use.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import { applyConfig, describeProblem } from "../config/apply.ts";
import { createConfigStore, type ConfigStore } from "../config/store.ts";
import { CoreClient } from "../core/client.ts";
import type { Notice } from "../core/types.ts";
import { NoticeStore, sentence } from "../shell/notices.ts";
import { TauriCarrier, type Carrier } from "../transport/carrier.ts";
import { createWindowStateStore, type WindowStateStore } from "../window/state.ts";
import { NO_LISTS, type SettingsLists, type SettingWrite } from "./model.ts";
import { SettingsRegistry } from "./registry.ts";

/** Which page the window is asked to show. */
export interface ShownPage {
  /** The page's id, or nothing for whichever comes first. */
  readonly page: string | null;
  /** Counts the times a page was asked for, so that asking again is noticed. */
  readonly asked: number;
  /** Asks for a page. */
  show(page: string | null): void;
}

/** Everything the settings window's components work with. */
export interface SettingsApp {
  readonly config: ConfigStore;
  readonly client: CoreClient;
  /** The configuration's schema, as the core sent it. */
  readonly schema: unknown;
  /** The machine's themes, fonts, shells and workspaces. */
  readonly lists: StoreApi<SettingsLists>;
  /** The pages that do not come from the schema. */
  readonly registry: SettingsRegistry;
  readonly notices: NoticeStore;
  /** What the window is doing on the desktop. */
  readonly windowState: WindowStateStore;
  /** The page the window was last asked to show. */
  readonly shown: StoreApi<ShownPage>;
  /** Asks the core to make a change to the user's file. */
  write(change: SettingWrite): Promise<void>;
  /** Opens the user's settings file for editing. */
  openFile(): void;
  /** Closes the window. */
  close(): void;
}

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** The page a window's address names after its `#`, if it names one. */
export function pageInAddress(hash: string): string | null {
  const named = hash.replace(/^#/, "").trim();
  return named === "" ? null : decodeURIComponent(named);
}

/**
 * Starts the settings window's page on `carrier`. The application's own page does
 * this once, on the carrier into the core; a test starts one on a carrier of its own.
 */
export async function startSettings(
  carrier: Carrier,
  address: string = globalThis.location?.hash ?? "",
): Promise<SettingsApp> {
  const config = createConfigStore();
  const notices = new NoticeStore();
  const windowState = createWindowStateStore();
  const registry = new SettingsRegistry();
  const lists = createStore<SettingsLists>()(() => NO_LISTS);
  const shown = createStore<ShownPage>()((set, get) => ({
    page: pageInAddress(address),
    asked: 0,
    show: (page) => set({ page, asked: get().asked + 1 }),
  }));

  // What the core sends before the page has its first copy is kept and followed once
  // it has one; a view the copy already holds is passed over there.
  let early: Notice[] | null = [];
  let onNotice = (notice: Notice): void => void early?.push(notice);

  const client = await CoreClient.open(carrier, {
    notice: (notice) => onNotice(notice),
    // This window answers nothing: it shows no terminals.
    request: () => undefined,
    failure: (error) =>
      notices.push("error", `The application can no longer be reached: ${error.message}`),
  });

  /** Asks again for the lists that change while the window is open. */
  const relist = (): void => {
    const take = <Key extends keyof SettingsLists>(key: Key, value: SettingsLists[Key]) =>
      lists.setState({ ...lists.getState(), [key]: value });
    // A list that cannot be had leaves the choices as they were.
    client.themesList().then(
      (themes) => take("themes", themes),
      () => undefined,
    );
    client.workspacesList().then(
      (listed) =>
        take(
          "workspaces",
          listed.workspaces.map((workspace) => workspace.name),
        ),
      () => undefined,
    );
  };

  const follow = (notice: Notice): void => {
    switch (notice.type) {
      case "config_changed":
        if (!config.getState().set(notice.view)) break;
        applyConfig(notice.view);
        // A theme may have been added to the folder the change came from.
        relist();
        break;
      case "config_error":
        notices.push(
          "error",
          `The configuration was not applied, and the one before it is still in use. ${notice.problems
            .map(describeProblem)
            .join(" ")}`,
        );
        break;
      case "window_state":
        windowState.getState().set(notice);
        break;
      case "settings_show":
        shown.getState().show(notice.section);
        break;
      case "notice":
        notices.push(notice.level, notice.message);
        break;
      default:
        // What is said to a window of terminals is not for this one.
        break;
    }
  };

  const [view, schema] = await Promise.all([client.configView(), client.configSchema()]);
  config.getState().set(view);
  applyConfig(view);

  // The lists fill the choices in as they arrive: the window does not wait for the
  // machine's fonts to be counted before it shows anything.
  const some = <Value>(asked: Promise<Value>, fallback: Value): Promise<Value> =>
    asked.catch(() => fallback);
  void Promise.all([
    some(client.themesList(), []),
    some(client.fontFamilies(), []),
    some(client.shellsList(), []),
    some(
      client.workspacesList().then((listed) => listed.workspaces.map((entry) => entry.name)),
      [],
    ),
  ]).then(([themes, fonts, shells, workspaces]) => {
    lists.setState({ themes, fonts, shells, workspaces });
  });

  client.windowState().then(
    (state) => windowState.getState().set(state),
    () => undefined,
  );

  onNotice = follow;
  for (const notice of early ?? []) follow(notice);
  early = null;

  return {
    config,
    client,
    schema,
    lists,
    registry,
    notices,
    windowState,
    shown,
    write: (change) =>
      change.op === "set"
        ? client.configSet(change.path, change.value)
        : client.configUnset(change.path),
    openFile: () => {
      client.configOpen("config").catch((error: unknown) => {
        notices.push("error", sentence(reason(error)));
      });
    },
    close: () => {
      client.windowControl("close").catch((error: unknown) => {
        notices.push("error", sentence(reason(error)));
      });
    },
  };
}

let app: Promise<SettingsApp> | undefined;

/** The settings window's page. The first call starts it; every call gets the same one. */
export function settingsApp(): Promise<SettingsApp> {
  app ??= startSettings(new TauriCarrier());
  return app;
}
