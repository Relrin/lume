/**
 * A view's way to its plugin's code and to the window around it.
 *
 * A plugin's panel or pane view is a page of the plugin's own in a sandboxed frame. It
 * can reach nothing but its own files. When it has loaded, the window hands it one end
 * of a channel, with a first message that says which view it is, what was kept for it
 * and how the window looks. Everything else goes over that channel:
 *
 * - calls to the plugin's code, which answers them with `lume.views.handle`;
 * - events the plugin's code sends with `lume.views.post`;
 * - what the view wants to find again when it next starts (JSON, 64 KiB at most);
 * - streams the plugin's code let the view use;
 * - keys the view has no use for, so the window's own keys work from inside it;
 * - the window's look whenever it changes, and word that the keyboard has come.
 *
 * `connect()` waits for the channel and gives back all of that as one object. A page
 * has ten seconds from loading to connect; `connect()` says the view is ready as soon as
 * it has the channel.
 */

import { keyToPass, type KeyLike, type PassedKey } from "./keys";
import { type StreamClock, type StreamPortLike, ViewStream } from "./streams";
import { applyPlace, applyTokens, type ThemeTokens } from "./tokens";

export { describeKey, editingKey, keyToPass, worthForwarding } from "./keys";
export type { KeyLike, PassedKey } from "./keys";
export { ACK_EVERY_MS, ViewStream, WRITE_MAX } from "./streams";
export type { PieceListener, StreamEnd } from "./streams";
export { applyPlace, applyTokens } from "./tokens";
export type { ThemeTokens } from "./tokens";

/** The version of the channel this kit speaks. */
export const BRIDGE_API = 1;

/** The most a view may have kept for it, as JSON. */
export const STATE_MAX = 64 * 1024;

/** The most one message to the window may be, as JSON. */
export const MESSAGE_MAX = 1024 * 1024;

/** How many of a view's calls may wait for their answers at once. */
export const CALLS_IN_FLIGHT_MAX = 64;

/** The longest a call's name may be. */
export const NAME_MAX = 128;

/**
 * A call or a stream that failed. `code` is the kind of failure: one of the
 * application's, or the name the plugin's own code gave it when it threw a
 * `LumeError` of its own. `data` is what that code said of the failure, or `null`.
 */
export class HostError<Data = unknown> extends Error {
  readonly code: string;
  readonly data: Data | null;

  constructor(code: string, message: string, data: Data | null = null) {
    super(message);
    this.name = "HostError";
    this.code = code;
    this.data = data;
  }
}

/** What the window first hands a view. */
export interface Hello {
  readonly lume: "connect";
  readonly api: number;
  /** The view as the plugin's code knows it. */
  readonly instance: string;
  /** The id the plugin is installed under. */
  readonly plugin: string;
  /** The view's name in the manifest. */
  readonly view: string;
  readonly place: ViewPlace;
  /** What was last kept for the view, or `null`. */
  readonly state: unknown;
  readonly tokens: ThemeTokens;
  readonly scheme: string;
}

/** The window's look. */
export interface Theme {
  readonly tokens: ThemeTokens;
  /** `light` or `dark`. */
  readonly scheme: string;
}

/** Something the window did not take, and why. */
export interface Refusal {
  readonly what: string;
  readonly message: string;
}

/** A view, connected. */
export interface View {
  /** The view as the plugin's code knows it: what `lume.views.post` is given. */
  readonly instance: string;
  /** The id the plugin is installed under. */
  readonly plugin: string;
  /** The view's name in the manifest. */
  readonly view: string;
  /** Where the view is shown. */
  readonly place: ViewPlace;

  readonly host: {
    /**
     * Calls the plugin's code, which answers under the same name with
     * `lume.views.handle`. Arguments and answers are JSON. Fails with a `HostError`.
     */
    call<T = unknown>(name: string, args?: unknown): Promise<T>;
    /**
     * Listens for an event the plugin's code sends this view with `lume.views.post`.
     * Returns what stops listening.
     */
    on<T = unknown>(event: string, listener: (payload: T) => void): () => void;
  };

  readonly state: {
    /** What is kept for the view: what it last set, or what it started with. */
    get<T = unknown>(): T | null;
    /**
     * Keeps `state` for the view, to be handed back when it next starts: after the
     * application was closed, after its pane was moved, after it was out of sight for
     * a while. JSON of 64 KiB at most; anything else throws.
     */
    set(state: unknown): void;
  };

  readonly streams: {
    /** Opens a stream by the grant the plugin's code made for this view. Once. */
    open(grant: string): Promise<ViewStream>;
  };

  readonly theme: {
    /** The window's look now. */
    current(): Theme;
    /** Listens for the look changing. Returns what stops listening. */
    onChange(listener: (theme: Theme) => void): () => void;
  };

  /**
   * Listens for the keyboard coming to the view (its pane was focused). Without a
   * listener the kit focuses the element marked `autofocus`, or the page.
   */
  onFocus(listener: () => void): () => void;
  /** Listens for the window refusing something the view sent. */
  onRefused(listener: (refusal: Refusal) => void): () => void;
  /** Passes a key on to the window, as the kit does for keys the view did not use. */
  passKey(key: PassedKey): void;
  /** Says the view failed. The window shows it as stopped, with a way to load it again. */
  fail(message: string): void;
}

/** The part of a port a view uses, so tests can stand one in. */
export interface PortLike {
  postMessage(message: unknown, transfer?: Transferable[]): void;
  onmessage: ((event: { data: unknown }) => void) | null;
}

/** What a view does to the page it is in; tests give their own. */
export interface ViewEffects {
  /** Sets the window's look on the page. */
  applyTheme?: ((theme: Theme) => void) | undefined;
  /** Takes the keyboard when no listener does. */
  focus?: (() => void) | undefined;
  /** The clock streams count by. */
  clock?: StreamClock | undefined;
}

function jsonSize(value: unknown): number | null {
  try {
    const text = JSON.stringify(value);
    return text === undefined ? null : text.length;
  } catch {
    return null;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Where a view is shown: in a pane, in a panel, or on the plugin's page in the settings
 * window, which shows the pane view its manifest marks with `settings = true`.
 */
export type ViewPlace = "pane" | "panel" | "settings";

/** Whether a message is the window's first to a view. */
export function isHello(data: unknown): data is Hello {
  return (
    isRecord(data) &&
    data["lume"] === "connect" &&
    typeof data["instance"] === "string" &&
    typeof data["plugin"] === "string" &&
    typeof data["view"] === "string" &&
    (data["place"] === "pane" || data["place"] === "panel" || data["place"] === "settings")
  );
}

type Waiting = {
  readonly resolve: (value: unknown) => void;
  readonly reject: (error: HostError) => void;
};

/**
 * Makes a view of the window's first message and the port that came with it, and says
 * it is ready. `connect()` does this for a page; tests call it with a port of their own.
 */
export function createView(hello: Hello, port: PortLike, effects: ViewEffects = {}): View {
  let nextId = 1;
  const waiting = new Map<number, Waiting>();
  const listeners = new Map<string, Set<(payload: unknown) => void>>();
  const themeListeners = new Set<(theme: Theme) => void>();
  const focusListeners = new Set<() => void>();
  const refusedListeners = new Set<(refusal: Refusal) => void>();
  let state: unknown = hello.state ?? null;
  let theme: Theme = {
    tokens: isRecord(hello.tokens) ? hello.tokens : {},
    scheme: typeof hello.scheme === "string" ? hello.scheme : "dark",
  };

  const post = (message: unknown) => port.postMessage(message, []);

  const ask = (message: Record<string, unknown>): Promise<unknown> => {
    if (waiting.size >= CALLS_IN_FLIGHT_MAX) {
      return Promise.reject(
        new HostError("busy", `a view waits for ${CALLS_IN_FLIGHT_MAX} answers at most`),
      );
    }
    const id = nextId++;
    return new Promise((resolve, reject) => {
      waiting.set(id, { resolve, reject });
      try {
        post({ ...message, id });
      } catch (error) {
        waiting.delete(id);
        reject(new HostError("failed", error instanceof Error ? error.message : String(error)));
      }
    });
  };

  const settle = (id: unknown): Waiting | null => {
    if (typeof id !== "number") return null;
    const found = waiting.get(id) ?? null;
    waiting.delete(id);
    return found;
  };

  port.onmessage = (event) => {
    const data = event.data;
    if (!isRecord(data)) return;
    switch (data["t"]) {
      case "result":
        settle(data["id"])?.resolve(data["value"] ?? null);
        return;
      case "opened":
        settle(data["id"])?.resolve(data);
        return;
      case "error": {
        const code = typeof data["code"] === "string" ? data["code"] : "failed";
        const message = typeof data["message"] === "string" ? data["message"] : "failed";
        settle(data["id"])?.reject(new HostError(code, message, data["data"] ?? null));
        return;
      }
      case "event": {
        const name = data["event"];
        if (typeof name !== "string") return;
        for (const listener of [...(listeners.get(name) ?? [])]) listener(data["payload"] ?? null);
        return;
      }
      case "tokens":
        theme = {
          tokens: isRecord(data["tokens"]) ? (data["tokens"] as ThemeTokens) : theme.tokens,
          scheme: typeof data["scheme"] === "string" ? data["scheme"] : theme.scheme,
        };
        effects.applyTheme?.(theme);
        for (const listener of [...themeListeners]) listener(theme);
        return;
      case "focus":
        if (focusListeners.size === 0) effects.focus?.();
        for (const listener of [...focusListeners]) listener();
        return;
      case "refused": {
        const refusal: Refusal = {
          what: typeof data["what"] === "string" ? data["what"] : "",
          message: typeof data["message"] === "string" ? data["message"] : "",
        };
        for (const listener of [...refusedListeners]) listener(refusal);
        return;
      }
      default:
        return;
    }
  };

  const listen = <T>(set: Set<T>, listener: T): (() => void) => {
    set.add(listener);
    return () => void set.delete(listener);
  };

  const view: View = {
    instance: hello.instance,
    plugin: hello.plugin,
    view: hello.view,
    place: hello.place,
    host: {
      call<T>(name: string, args?: unknown): Promise<T> {
        if (typeof name !== "string" || name.length === 0 || name.length > NAME_MAX) {
          return Promise.reject(
            new HostError("invalid", `a call's name is 1 to ${NAME_MAX} characters`),
          );
        }
        const size = jsonSize(args ?? null);
        if (size === null || size > MESSAGE_MAX - 1024) {
          return Promise.reject(
            new HostError("invalid", "a call's arguments are JSON of 1 MiB at most"),
          );
        }
        return ask({ t: "call", name, args: args ?? null }) as Promise<T>;
      },
      on<T>(event: string, listener: (payload: T) => void): () => void {
        const set = listeners.get(event) ?? new Set();
        listeners.set(event, set);
        const held = listener as (payload: unknown) => void;
        set.add(held);
        return () => {
          set.delete(held);
          if (set.size === 0) listeners.delete(event);
        };
      },
    },
    state: {
      get<T>(): T | null {
        return state as T | null;
      },
      set(next: unknown): void {
        const size = jsonSize(next ?? null);
        if (size === null) throw new TypeError("what a view keeps must be JSON");
        if (size > STATE_MAX) {
          throw new RangeError(`a view keeps ${STATE_MAX / 1024} KiB at most`);
        }
        state = next ?? null;
        post({ t: "state", state });
      },
    },
    streams: {
      async open(grant: string): Promise<ViewStream> {
        if (typeof grant !== "string" || grant.length === 0 || grant.length > 64) {
          throw new HostError("not_found", "there is no such grant");
        }
        const opened = (await ask({ t: "stream", grant })) as Record<string, unknown>;
        return new ViewStream(
          opened["port"] as StreamPortLike,
          { readable: opened["readable"] === true, writable: opened["writable"] === true },
          effects.clock,
        );
      },
    },
    theme: {
      current: () => theme,
      onChange: (listener) => listen(themeListeners, listener),
    },
    onFocus: (listener) => listen(focusListeners, listener),
    onRefused: (listener) => listen(refusedListeners, listener),
    passKey(key: PassedKey): void {
      post({ t: "key", key: key.key, code: key.code, mods: [...key.mods] });
    },
    fail(message: string): void {
      post({ t: "error", message: String(message).slice(0, 1000) });
    },
  };

  effects.applyTheme?.(theme);
  post({ t: "ready" });
  return view;
}

/** How `connect()` may be told to behave. */
export interface ConnectOptions {
  /**
   * Whether the window's look is set on the page as CSS variables, when the view
   * starts and whenever it changes. On unless told otherwise.
   */
  readonly applyTokens?: boolean | undefined;
  /**
   * Whether keys the view did not use are passed on to the window. On unless told
   * otherwise; a view that turns it off passes keys on itself with `passKey`.
   */
  readonly forwardKeys?: boolean | undefined;
}

/** Whether the keyboard is in something that is typed into. */
function inField(target: unknown): boolean {
  if (!isRecord(target)) return false;
  const element = target as unknown as {
    tagName?: string;
    isContentEditable?: boolean;
    type?: string;
  };
  if (element.isContentEditable === true) return true;
  const tag = (element.tagName ?? "").toLowerCase();
  if (tag === "textarea") return true;
  if (tag !== "input") return false;
  return !["button", "checkbox", "radio", "range", "color", "file", "submit", "reset"].includes(
    (element.type ?? "text").toLowerCase(),
  );
}

let connecting: Promise<View> | null = null;

/**
 * Connects the page to the window it is shown in. Settles once the window has handed
 * over the channel, which it does when the page has loaded; calling it again gives the
 * same view. Call it from the page's own script as it starts: a page that has not
 * connected ten seconds after loading is shown as stopped.
 */
export function connect(options: ConnectOptions = {}): Promise<View> {
  if (connecting !== null) return connecting;
  connecting = new Promise<View>((resolve) => {
    const received = (event: MessageEvent) => {
      const port = event.ports[0];
      if (!isHello(event.data) || event.ports.length !== 1 || port === undefined) return;
      window.removeEventListener("message", received);
      if (options.applyTokens !== false) applyPlace(event.data.place);
      const view = createView(event.data, port as unknown as PortLike, {
        applyTheme:
          options.applyTokens === false
            ? undefined
            : (theme) => applyTokens(theme.tokens, theme.scheme),
        focus: () => {
          const wanted = document.querySelector<HTMLElement>("[autofocus]");
          if (wanted) wanted.focus();
          else window.focus();
        },
      });
      if (options.forwardKeys !== false) {
        // Read as the key leaves the page, so whatever the view does with a key, and
        // says so with preventDefault, is done first.
        window.addEventListener("keydown", (key) => {
          const passed = keyToPass(key as KeyLike, inField(key.target));
          if (passed !== null) view.passKey(passed);
        });
      }
      resolve(view);
    };
    window.addEventListener("message", received);
  });
  return connecting;
}
