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
  CommandSpec,
  ConfigView,
  ControlMessage,
  DetachedSessions,
  LayoutApplied,
  LayoutView,
  Notice,
  PaneStart,
  ProfileList,
  SessionsStarted,
  ShellEntry,
  ThemeEntry,
  WindowAction,
  WindowState,
  WorkspaceList,
} from "./types.ts";

/** What a page does with what the core sends it. */
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
  configOpen(file: "config" | "workspace" = "config"): Promise<void> {
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

  /** The fixed-width fonts installed on this machine. */
  fontFamilies(): Promise<string[]> {
    return this.transport.command("font_families");
  }

  /**
   * The picture the configuration puts behind the panes, as the bytes of its file.
   * Empty when there is none.
   */
  async backgroundImage(): Promise<Uint8Array> {
    return bytesOf(await this.transport.command<unknown>("background_image"));
  }

  /** Every command there is. */
  commandsList(): Promise<CommandSpec[]> {
    return this.transport.command("commands_list");
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
