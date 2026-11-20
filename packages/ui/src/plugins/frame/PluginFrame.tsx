/**
 * A plugin's own page, shown in a sandboxed frame: a pane view, or a panel that shows a
 * page.
 *
 * The frame may run scripts and nothing else: it has an origin that matches no other
 * page, so it holds no storage and cannot reach this page, and it can neither navigate
 * the window nor open others. Its files come from the plugin's package only, under a
 * policy that lets it connect to nothing. Once it has loaded, it is handed one end of a
 * channel (see `bridge.ts`), its only way to its plugin's code.
 *
 * A frame that loads a second time navigated away from its page, which a view never
 * needs to do; it is taken down rather than handed another channel. One that does not
 * say it is ready within ten seconds, that fails, or that breaks the bridge's rules, is
 * shown as stopped, with a button to load it again. The frames of a plugin that is
 * being worked on are loaded again whenever its files change.
 *
 * A view may ask to have something kept for it. That is handed back to it whenever it
 * starts again: after the application was closed, after its pane was moved to where a
 * frame cannot follow, and after it was let go of. A frame that is out of sight is let
 * go of once it has been so for half a minute, and started again from what it kept
 * when it is next shown; until then it keeps running, so going to another tab and back
 * finds it as it was.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "zustand";

import { CSS_VARIABLES } from "@lume/tokens";

import { useShell } from "../../shell/context.ts";
import { FrameStreams, STREAM_WINDOW } from "../../transport/view-streams.ts";
import { FrameBridge, type Hello, pageUrl, pluginBase } from "./bridge.ts";
import { keepFrame, keepFrameReload, keepPaneFrame, nextViewName } from "./registry.ts";

/** How long a frame has to say it is ready. */
export const READY_WAIT_MS = 10_000;

/** How long a frame that is out of sight is kept running. */
export const HIDDEN_UNLOAD_MS = 30_000;

/** How long after a view last asked to have something kept it is handed on. */
export const STATE_SETTLE_MS = 300;

type Shown =
  | { readonly kind: "loading" }
  | { readonly kind: "ready" }
  | { readonly kind: "stopped"; readonly reason: string };

/** The window's look as frames are given it: every token's CSS variable and value. */
export function frameTokens(root: Element = document.documentElement): Record<string, string> {
  const style = getComputedStyle(root);
  const tokens: Record<string, string> = {};
  for (const variable of Object.values(CSS_VARIABLES)) {
    tokens[variable] = style.getPropertyValue(variable).trim();
  }
  return tokens;
}

export interface PluginFrameProps {
  /** The plugin's install. */
  readonly plugin: string;
  /** The view's name in its manifest. */
  readonly view: string;
  readonly place: "pane" | "panel";
  /** The page, as a path in the package. */
  readonly entry: string;
  /** What the frame is called, for assistive technology. */
  readonly title: string;
  /** What was last kept for the view, handed to it when it starts. */
  readonly state?: unknown;
  /** The pane or the panel the view is shown in, which what it keeps is kept with. */
  readonly host?: string;
  /** Whether the frame can be seen. One that cannot for long is let go of. */
  readonly shown?: boolean;
  /** Called when the keyboard goes into the frame, as when it is clicked in. */
  readonly onFocusIn?: () => void;
}

/** A plugin's page in a sandboxed frame, with the bridge to its code. */
export function PluginFrame(props: PluginFrameProps) {
  const app = useShell();
  const [attempt, setAttempt] = useState(0);
  const shown = props.shown ?? true;
  // A frame that starts out of sight is not loaded until it is first shown.
  const [asleep, setAsleep] = useState(!shown);
  // What the view last asked to have kept, which outlives each run of its frame.
  const kept = useRef<{ readonly state: unknown } | null>(null);

  useEffect(() => {
    if (shown) {
      setAsleep(false);
      return undefined;
    }
    const timer = setTimeout(() => setAsleep(true), HIDDEN_UNLOAD_MS);
    return () => clearTimeout(timer);
  }, [shown]);

  const keep = useCallback((state: unknown) => {
    kept.current = { state };
  }, []);

  // Loaded again from what it kept when its plugin's files change; one that is asleep
  // loads the new files when it is next shown anyway.
  const { plugin } = props;
  useEffect(() => keepFrameReload(plugin, () => setAttempt((n) => n + 1)), [plugin]);

  if (asleep) return <div className="plugin-frame" data-shown="asleep" />;
  return (
    <FrameRun
      key={attempt}
      {...props}
      state={kept.current ? kept.current.state : props.state}
      app={app}
      keep={keep}
      again={() => setAttempt((n) => n + 1)}
    />
  );
}

function FrameRun({
  plugin,
  view,
  place,
  entry,
  title,
  state,
  host,
  onFocusIn,
  app,
  keep,
  again,
}: PluginFrameProps & {
  app: ReturnType<typeof useShell>;
  keep: (state: unknown) => void;
  again: () => void;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [shown, setShown] = useState<Shown>({ kind: "loading" });
  const [instance, setInstance] = useState<{ page: string; id: string } | null>(null);
  const bridge = useRef<FrameBridge | null>(null);
  const loads = useRef(0);
  const firstState = useRef(state);
  const focusIn = useRef(onFocusIn);
  focusIn.current = onFocusIn;
  const scheme = useStore(app.config, (store) => store.view?.theme.scheme ?? "dark");

  const opened = useRef<string | null>(null);
  const stop = useCallback(
    (reason: string) => {
      bridge.current?.close();
      bridge.current = null;
      setShown({ kind: "stopped", reason });
      // The plugin's code hears that its view is gone, as it is.
      if (opened.current !== null) {
        void app.client.viewClosed(opened.current).catch(() => undefined);
      }
    },
    [app],
  );

  // What the view asks to have kept is handed on once it has stopped changing, and
  // before the view is closed.
  const saving = useRef<{
    page: string | null;
    pending: { readonly state: unknown } | null;
    timer: ReturnType<typeof setTimeout> | null;
  }>({ page: null, pending: null, timer: null });
  const flush = useCallback((): Promise<void> => {
    const save = saving.current;
    if (save.timer !== null) clearTimeout(save.timer);
    save.timer = null;
    const pending = save.pending;
    save.pending = null;
    if (pending === null || save.page === null || host === undefined) return Promise.resolve();
    return app.client.viewState(save.page, pending.state).catch(() => undefined);
  }, [app, host]);
  const keepState = useCallback(
    (next: unknown) => {
      keep(next);
      const save = saving.current;
      save.pending = { state: next };
      if (save.timer !== null) clearTimeout(save.timer);
      save.timer = setTimeout(() => void flush(), STATE_SETTLE_MS);
    },
    [keep, flush],
  );

  // The view is opened with the application before its page loads, so its first call
  // finds it.
  useEffect(() => {
    const page = nextViewName();
    const save = saving.current;
    let open = true;
    app.client.viewOpened(page, plugin, view, place, host ?? null).then(
      (id) => {
        if (!open) return;
        save.page = page;
        opened.current = page;
        setInstance({ page, id });
      },
      (error: unknown) => {
        if (open) stop(error instanceof Error ? error.message : String(error));
      },
    );
    return () => {
      open = false;
      // What it kept goes first: the application takes it only from an open view.
      void flush().then(() => app.client.viewClosed(page).catch(() => undefined));
    };
  }, [app, plugin, view, place, host, stop, flush]);

  // A page that never says it is ready is taken for stopped.
  useEffect(() => {
    if (shown.kind !== "loading" || instance === null) return undefined;
    const timer = setTimeout(() => stop("it did not start in time"), READY_WAIT_MS);
    return () => clearTimeout(timer);
  }, [shown.kind, instance, stop]);

  useEffect(() => () => bridge.current?.close(), []);

  // The look follows the window's.
  useEffect(() => {
    bridge.current?.tokens(frameTokens(), scheme);
  }, [scheme, app.config]);
  useEffect(
    () =>
      app.config.subscribe((now, before) => {
        if (now.view !== before.view) {
          bridge.current?.tokens(frameTokens(), now.view?.theme.scheme ?? "dark");
        }
      }),
    [app.config],
  );

  // A pane's frame that is ready can be given the keyboard, and says when it took it.
  const ready = shown.kind === "ready";
  useEffect(() => {
    if (!ready || place !== "pane" || host === undefined) return undefined;
    return keepPaneFrame(host, () => {
      frame.current?.focus();
      bridge.current?.focus();
    });
  }, [ready, place, host]);
  useEffect(() => {
    // A click in a frame is the frame's own: all this page sees of it is that the
    // keyboard left for one of its frames.
    const left = () => {
      if (document.activeElement === frame.current) focusIn.current?.();
    };
    window.addEventListener("blur", left);
    return () => window.removeEventListener("blur", left);
  }, []);

  const loaded = () => {
    const element = frame.current;
    const target = element?.contentWindow;
    if (!element || !target || instance === null) return;
    loads.current += 1;
    if (loads.current > 1) {
      stop("it tried to leave its page");
      element.removeAttribute("src");
      return;
    }
    const channel = new MessageChannel();
    const streams = new FrameStreams((grant) =>
      app.client.transport.pipe(instance.page, grant, STREAM_WINDOW),
    );
    const made = new FrameBridge(channel.port1, {
      call: (name, args) => app.client.viewCall(instance.page, name, args),
      ready: () => setShown((now) => (now.kind === "loading" ? { kind: "ready" } : now)),
      state: keepState,
      key: (key) => app.forwardKey(key, place === "pane" ? host : undefined),
      stream: (grant) =>
        streams.open(grant, () => {
          const ends = new MessageChannel();
          return { mine: ends.port1, theirs: ends.port2 };
        }),
      stop: (reason) => {
        streams.close();
        stop(reason);
      },
    });
    bridge.current = made;
    const release = keepFrame(instance.page, made);
    const closing = made.close.bind(made);
    made.close = () => {
      release();
      streams.close();
      closing();
    };
    const hello: Hello = {
      lume: "connect",
      api: 1,
      instance: instance.id,
      plugin,
      view,
      place,
      state: firstState.current ?? null,
      tokens: frameTokens(),
      scheme,
    };
    // The frame's origin matches nothing, so it cannot be named here; the window it is
    // sent to is the frame this page made.
    target.postMessage(hello, "*", [channel.port2]);
  };

  return (
    <div className="plugin-frame" data-shown={shown.kind}>
      {instance !== null && shown.kind !== "stopped" && (
        <iframe
          ref={frame}
          className="plugin-frame-page"
          title={title}
          src={pageUrl(pluginBase(), plugin, entry)}
          sandbox="allow-scripts"
          referrerPolicy="no-referrer"
          allow=""
          onLoad={loaded}
        />
      )}
      {shown.kind === "loading" && (
        <div className="plugin-frame-note" role="status">
          <span className="plugin-frame-spinner" aria-hidden="true" />
        </div>
      )}
      {shown.kind === "stopped" && (
        <div className="plugin-frame-note" role="alert">
          <p>
            {title} stopped: {shown.reason}.
          </p>
          <button type="button" className="control plugin-frame-again" onClick={again}>
            Load it again
          </button>
        </div>
      )}
    </div>
  );
}
