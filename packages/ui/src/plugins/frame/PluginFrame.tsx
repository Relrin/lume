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
 * shown as stopped, with a button to load it again.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "zustand";

import { CSS_VARIABLES } from "@lume/tokens";

import { useShell } from "../../shell/context.ts";
import { FrameBridge, type Hello, pageUrl, pluginBase } from "./bridge.ts";
import { keepFrame, nextViewName } from "./registry.ts";

/** How long a frame has to say it is ready. */
export const READY_WAIT_MS = 10_000;

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
  /** What the view last kept, handed to it when it starts. */
  readonly state?: unknown;
  /** Keeps what the view wants to find again. */
  readonly onState?: (state: unknown) => void;
}

/** A plugin's page in a sandboxed frame, with the bridge to its code. */
export function PluginFrame(props: PluginFrameProps) {
  const app = useShell();
  const [attempt, setAttempt] = useState(0);
  return <FrameRun key={attempt} {...props} app={app} again={() => setAttempt((n) => n + 1)} />;
}

function FrameRun({
  plugin,
  view,
  place,
  entry,
  title,
  state,
  onState,
  app,
  again,
}: PluginFrameProps & { app: ReturnType<typeof useShell>; again: () => void }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [shown, setShown] = useState<Shown>({ kind: "loading" });
  const [instance, setInstance] = useState<{ page: string; id: string } | null>(null);
  const bridge = useRef<FrameBridge | null>(null);
  const loads = useRef(0);
  const firstState = useRef(state);
  const keeping = useRef(onState);
  keeping.current = onState;
  const scheme = useStore(app.config, (store) => store.view?.theme.scheme ?? "dark");

  const stop = useCallback((reason: string) => {
    bridge.current?.close();
    bridge.current = null;
    setShown({ kind: "stopped", reason });
  }, []);

  // The view is opened with the application before its page loads, so its first call
  // finds it.
  useEffect(() => {
    const page = nextViewName();
    let open = true;
    app.client.viewOpened(page, plugin, view, place).then(
      (id) => {
        if (open) setInstance({ page, id });
      },
      (error: unknown) => {
        if (open) stop(error instanceof Error ? error.message : String(error));
      },
    );
    return () => {
      open = false;
      void app.client.viewClosed(page).catch(() => undefined);
    };
  }, [app, plugin, view, place, stop]);

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
    const made = new FrameBridge(channel.port1, {
      call: (name, args) => app.client.viewCall(instance.page, name, args),
      ready: () => setShown((now) => (now.kind === "loading" ? { kind: "ready" } : now)),
      state: (kept) => keeping.current?.(kept),
      // Keys a view passes on are taken up with the window's keys separately.
      key: () => undefined,
      stop,
    });
    bridge.current = made;
    const release = keepFrame(instance.page, made);
    const closing = made.close.bind(made);
    made.close = () => {
      release();
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
