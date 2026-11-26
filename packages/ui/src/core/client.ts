/**
 * The page's way to the core for everything that is not a session's bytes: the layout,
 * the configuration, the commands, and what the core says or asks unprompted.
 *
 * It shares the page's one carrier with the session transport, so a notice about a new
 * pane and the first output of that pane's session arrive in the order the core sent
 * them.
 */

import type { Intent, PaneId } from "../layout/types.ts";
import type { Carrier } from "../transport/carrier.ts";
import { TransportClient } from "../transport/client.ts";
import type {
  AboutInfo,
  CommandSpec,
  ConfigView,
  ControlMessage,
  DetachedSessions,
  FolderList,
  HostProbe,
  LayoutApplied,
  LayoutView,
  ListedProfile,
  Notice,
  PaneStart,
  ProfileList,
  RexAddress,
  RexHosts,
  RexTried,
  SessionsStarted,
  ShellEntry,
  SshHost,
  ThemeEntry,
  UiContributions,
  WindowAction,
  WindowState,
  WorkspaceList,
} from "./types.ts";
import type {
  InstalledPlugin,
  Inspection,
  InstallPreview,
  UpdateCurrent,
} from "../settings/plugins.ts";

/** What a page does with what the core sends it. */
/**
 * One of the user's files the core opens for editing: the configuration, the file of the
 * workspace in use, or a layout or theme file by its name.
 */
export type ConfigFile = "config" | "workspace" | `layout:${string}` | `theme:${string}`;

export interface CoreEvents {
  /** Something the page keeps a copy of has changed, or there is something to say. */
  notice(notice: Notice): void;
  /** The core asks something only the page can answer. The whole message is handed on. */
  request(message: unknown): void;
  /** The core can no longer be reached: this page's carrier was replaced, or broke. */
  failure(error: Error): void;
}

/**
 * Bytes as the core hands them over: a buffer where the webview can fetch one, a list
 * of numbers where it cannot, and nothing at all for no bytes.
 */
function bytesOf(raw: unknown): Uint8Array {
  if (raw instanceof Uint8Array) return raw;
  if (raw instanceof ArrayBuffer) return new Uint8Array(raw);
  if (Array.isArray(raw)) return Uint8Array.from(raw as number[]);
  return new Uint8Array(0);
}

function isNotice(message: unknown): message is Extract<ControlMessage, { notify: Notice }> {
  if (typeof message !== "object" || message === null || !("notify" in message)) return false;
  const notice = message.notify;
  return typeof notice === "object" && notice !== null && "type" in notice;
}

/** The core, as a page calls it. */
export class CoreClient {
  /** The session transport on the same carrier. */
  readonly transport: TransportClient;

  private constructor(transport: TransportClient) {
    this.transport = transport;
  }

  /** Opens `carrier` and starts handing what the core sends to `events`. */
  static async open(carrier: Carrier, events: CoreEvents): Promise<CoreClient> {
    const transport = await TransportClient.open(carrier, {
      onFailure: (error) => events.failure(error),
      onControl: (message) => {
        if (isNotice(message)) events.notice(message.notify);
        else events.request(message);
      },
    });
    return new CoreClient(transport);
  }

  /** Opens the tabs of a layout file in this window. */
  layoutsOpen(id: string): Promise<{ readonly tab?: string; readonly group?: string }> {
    return this.transport.command("layouts_open", { id });
  }

  /** The layout as it is now, and which of its windows this page is. */
  layoutView(): Promise<LayoutView> {
    return this.transport.command("layout_view");
  }

  /** Asks for the layout to be changed. The change itself arrives as a notice. */
  layoutApply(intent: Intent): Promise<LayoutApplied> {
    return this.transport.command("layout_apply", { intent });
  }

  /** Has sessions started for panes of this window that have none. */
  sessionsStart(panes: readonly PaneStart[]): Promise<SessionsStarted> {
    return this.transport.command("sessions_start", { panes });
  }

  /** Which of this window's `panes` run something that closing them would end. */
  panesBusy(panes: readonly PaneId[]): Promise<PaneId[]> {
    return this.transport.command("panes_busy", { panes });
  }

  /**
   * How long a connection to the machine a profile reaches over SSH takes to open, or
   * why it was not measured.
   */
  hostProbe(profile: string): Promise<HostProbe> {
    return this.transport.command("host_probe", { profile });
  }

  /** The sessions the session host runs that no pane of any workspace shows. */
  sessionsDetached(): Promise<DetachedSessions> {
    return this.transport.command("sessions_detached");
  }

  /** The configuration in use. */
  configView(): Promise<ConfigView> {
    return this.transport.command("config_view");
  }

  /** Sets one setting in the user's file. */
  configSet(path: string, value: unknown): Promise<void> {
    return this.transport.command("config_set", { path, value });
  }

  /** Takes one setting out of the user's file. */
  configUnset(path: string): Promise<void> {
    return this.transport.command("config_unset", { path });
  }

  /** Opens one of the user's settings files for editing. */
  configOpen(file: ConfigFile = "config"): Promise<void> {
    return this.transport.command("config_open", { file });
  }

  /** What the settings are and may be: a JSON Schema of the configuration. */
  configSchema(): Promise<unknown> {
    return this.transport.command("config_schema");
  }

  /** The themes there are to choose from. */
  themesList(): Promise<ThemeEntry[]> {
    return this.transport.command("themes_list");
  }

  /** The shells found on this machine. */
  shellsList(): Promise<ShellEntry[]> {
    return this.transport.command("shells_list");
  }

  /** The ways there are to start a terminal. */
  profilesList(): Promise<ProfileList> {
    return this.transport.command("profiles_list");
  }

  /** The Rex servers there are, and how each stands. */
  rexHosts(): Promise<RexHosts> {
    return this.transport.command("rex_hosts");
  }

  /**
   * The same, for something that lists them and is on show: the core looks whether a
   * server runs on this machine, and how near those are that are connected.
   */
  rexHostsLook(): Promise<RexHosts> {
    return this.transport.command("rex_hosts_look");
  }

  /**
   * How an address of a Rex server is read, with the server already reached there if
   * it is another than the one called `except`.
   */
  rexRead(address: string, except: string | null): Promise<RexAddress> {
    return this.transport.command("rex_read", { address, except });
  }

  /** How long the machine at an address takes to answer, or why that is not known. */
  rexReach(address: string): Promise<HostProbe> {
    return this.transport.command("rex_reach", { address });
  }

  /**
   * Connects to the server at an address, to see what it is, and lets go of it again.
   * `agreed` is whether the user agreed to a connection that is not encrypted, and
   * `attempt` a number to end it by.
   */
  rexTry(address: string, agreed: boolean, attempt: number): Promise<RexTried> {
    return this.transport.command("rex_try", { address, agreed, attempt });
  }

  /** Ends a try that nobody waits for any more. */
  tryEnd(attempt: number): Promise<void> {
    return this.transport.command("try_end", { attempt });
  }

  /** Writes a Rex server into the user's file, in place of the one called `was` if any. */
  rexHostPut(was: string | null, label: string, address: string): Promise<void> {
    return this.transport.command("rex_host_put", { was, label, address });
  }

  /** Takes a Rex server out of the user's file. */
  rexHostRemove(label: string): Promise<void> {
    return this.transport.command("rex_host_remove", { label });
  }

  /** How long a machine takes to answer on its SSH port, or why that is not known. */
  sshReach(host: SshHost): Promise<HostProbe> {
    return this.transport.command("ssh_reach", { host });
  }

  /**
   * Signs in to a machine over SSH and leaves again. It fails with the reason it could
   * not. `attempt` is a number to end it by.
   */
  sshTry(host: SshHost, attempt: number): Promise<void> {
    return this.transport.command("ssh_try", { host, attempt });
  }

  /** Writes a profile for a machine reached over SSH, in place of the one whose id is
   *  given if any. Gives its id. */
  sshProfilePut(id: string | null, name: string, host: SshHost): Promise<string> {
    return this.transport.command("ssh_profile_put", { id, name, host });
  }

  /** Takes a profile of the user's own out of their file. */
  profileRemove(id: string): Promise<void> {
    return this.transport.command("profile_remove", { id });
  }

  /** Says of profiles that were found whether they are listed and where they start. */
  profilesListed(changes: readonly ListedProfile[]): Promise<ProfileList> {
    return this.transport.command("profiles_listed", { changes });
  }

  /** The fixed-width fonts installed on this machine. */
  fontFamilies(): Promise<string[]> {
    return this.transport.command("font_families");
  }

  /** Every font installed on this machine, for the interface to be drawn in. */
  interfaceFontFamilies(): Promise<string[]> {
    return this.transport.command("font_families", { all: true });
  }

  /**
   * The picture the configuration puts behind the panes, as the bytes of its file.
   * Empty when there is none.
   */
  async backgroundImage(): Promise<Uint8Array> {
    return bytesOf(await this.transport.command<unknown>("background_image"));
  }

  /** What the application is: its version and its parts', and the Rex server's. */
  aboutInfo(): Promise<AboutInfo> {
    return this.transport.command("about_info");
  }

  /** Every command there is. */
  commandsList(): Promise<CommandSpec[]> {
    return this.transport.command("commands_list");
  }

  /** What the plugins add to the window, with what their code shows in it now. */
  uiContributions(): Promise<UiContributions> {
    return this.transport.command("ui_contributions");
  }

  /** Says which of the plugins' things this window has on screen, by their ids. */
  uiVisible(items: readonly string[]): Promise<void> {
    return this.transport.command("ui_visible", { items });
  }

  /** The plugins' pictures and shaders the core has, of those named by their hashes. */
  uiAssets(hashes: readonly string[]): Promise<Record<string, string>> {
    return this.transport.command("ui_assets", { hashes });
  }

  /** Asks a plugin for the rows beneath a row of its panel that the user opened. */
  uiPanelExpand(panel: string, path: readonly string[]): Promise<void> {
    return this.transport.command("ui_panel_expand", { panel, path });
  }

  /**
   * Opens one of a plugin's views in this page, under the page's own name for it; gives
   * the instance the plugin's code knows it by.
   */
  viewOpened(
    view: string,
    plugin: string,
    name: string,
    place: "pane" | "panel" | "settings",
    host: string | null,
  ): Promise<string> {
    return this.transport.command("view_opened", { view, plugin, name, place, host });
  }

  /** Keeps what one of the page's views wants to find again when it next starts. */
  viewState(view: string, state: unknown): Promise<void> {
    return this.transport.command("view_state", { view, state: state ?? null });
  }

  /** Closes one of the page's views. */
  viewClosed(view: string): Promise<void> {
    return this.transport.command("view_closed", { view });
  }

  /** A call one of the page's views makes of its plugin's code. */
  viewCall(view: string, name: string, args: unknown): Promise<unknown> {
    return this.transport.command("view_call", { view, name, args: args ?? null });
  }

  /** Every plugin installed, in the order they load in, with how each fares. */
  pluginsList(): Promise<{ plugins: InstalledPlugin[] }> {
    return this.transport.command("plugins_list");
  }

  /** Turns a plugin on or off. */
  pluginSetEnabled(id: string, enabled: boolean): Promise<void> {
    return this.transport.command("plugin_set_enabled", { id, enabled });
  }

  /** Removes a plugin; its data is kept. */
  pluginRemove(id: string): Promise<void> {
    return this.transport.command("plugin_remove", { id });
  }

  /** Takes back what a plugin may run on another machine. */
  pluginRevokeMachine(id: string, machine: string): Promise<void> {
    return this.transport.command("plugin_revoke_machine", { id, machine });
  }

  /** Moves a plugin one place earlier or later in the order plugins load in. */
  pluginMove(id: string, earlier: boolean): Promise<void> {
    return this.transport.command("plugin_move", { id, earlier });
  }

  /** Copies and checks the package in a folder, and says what installing it would do. */
  pluginInspect(folder: string, installAs: string | null): Promise<Inspection> {
    return this.transport.command("plugin_inspect", { folder, installAs });
  }

  /** Installs a package the user agreed to, by the token its inspection gave. */
  pluginInspectGit(
    source: string,
    path: string | null,
    installAs: string | null,
  ): Promise<Inspection> {
    return this.transport.command("plugin_inspect_git", { source, path, installAs });
  }

  /** Looks in the repository a plugin was installed from for something newer. */
  pluginCheckUpdate(id: string): Promise<InstallPreview | UpdateCurrent> {
    return this.transport.command("plugin_check_update", { id });
  }

  pluginInstall(token: number): Promise<{ id: string }> {
    return this.transport.command("plugin_install", { token });
  }

  /** Lets go of a package the user did not install. */
  pluginCancel(token: number): Promise<void> {
    return this.transport.command("plugin_cancel", { token });
  }

  /** Runs a command that is carried out in the core. */
  commandRun(id: string, args: unknown = null): Promise<void> {
    return this.transport.command("command_run", { id, args });
  }

  /** Starts moving this page's window with the pointer that is pressed on it. */
  windowDrag(): Promise<void> {
    return this.transport.command("window_drag");
  }

  /**
   * Minimises or closes this page's window, or maximises it or puts it back.
   * `confirmed` says the user has already agreed to a close that asks first.
   */
  windowControl(action: WindowAction, confirmed = false): Promise<void> {
    return this.transport.command(
      "window_control",
      confirmed ? { action, confirmed: true } : { action },
    );
  }

  /** What this page's window is doing on the desktop. */
  windowState(): Promise<WindowState> {
    return this.transport.command("window_state");
  }

  /** The workspaces there are, and which of them is on show. */
  workspacesList(): Promise<WorkspaceList> {
    return this.transport.command("workspaces_list");
  }

  /** The folders inside a folder on this machine; `~` is the home folder. */
  foldersList(path: string): Promise<FolderList> {
    return this.transport.command("folders_list", { path });
  }

  /** The text on the system clipboard, or nothing when it holds no text. */
  clipboardRead(): Promise<string | null> {
    return this.transport.command("clipboard_read");
  }

  /** Puts text on the system clipboard. */
  clipboardWrite(text: string): Promise<void> {
    return this.transport.command("clipboard_write", { text });
  }

  /** Opens a link the user asked for. The core decides again whether it may be opened. */
  openLink(url: string): Promise<void> {
    return this.transport.command("open_link", { url });
  }

  /** Answers a request the core sent. */
  answer(id: number, result: unknown): Promise<void> {
    return this.transport.answer(id, result);
  }
}
