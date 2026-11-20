/**
 * The view for React: a provider that holds it, and hooks that read from it.
 *
 * ```tsx
 * const view = await connect();
 * createRoot(document.getElementById("root")!).render(
 *   <ViewProvider view={view}>
 *     <App />
 *   </ViewProvider>,
 * );
 * ```
 */

import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";

import type { HostError, Theme, View } from "./bridge";
import type { PieceListener, StreamEnd, ViewStream } from "./streams";

const ViewContext = createContext<View | null>(null);

/** Gives the components below it the connected view. */
export function ViewProvider({ view, children }: { view: View; children?: ReactNode }) {
  return <ViewContext.Provider value={view}>{children}</ViewContext.Provider>;
}

/** The connected view. Only inside a `ViewProvider`. */
export function useView(): View {
  const view = useContext(ViewContext);
  if (view === null) throw new Error("useView is used inside a ViewProvider");
  return view;
}

/**
 * Listens, while the component is shown, for an event the plugin's code sends this
 * view with `lume.views.post`. The listener may change from one render to the next.
 */
export function useHostEvent<T = unknown>(event: string, listener: (payload: T) => void): void {
  const view = useView();
  const latest = useRef(listener);
  latest.current = listener;
  useEffect(() => view.host.on<T>(event, (payload) => latest.current(payload)), [view, event]);
}

/**
 * A value that is kept for the view and handed back when it next starts, like
 * `useState` that outlives the view. `initial` is used when nothing was kept. The value
 * is the view's whole state: use one `useViewState` per view, with an object in it.
 */
export function useViewState<T>(initial: T): [T, (next: T | ((before: T) => T)) => void] {
  const view = useView();
  const [value, setValue] = useState<T>(() => view.state.get<T>() ?? initial);
  const held = useRef(value);
  const set = useCallback(
    (next: T | ((before: T) => T)) => {
      const made = typeof next === "function" ? (next as (before: T) => T)(held.current) : next;
      held.current = made;
      view.state.set(made);
      setValue(made);
    },
    [view],
  );
  return [value, set];
}

/** The window's look, followed as it changes. */
export function useTheme(): Theme {
  const view = useView();
  return useSyncExternalStore(view.theme.onChange, view.theme.current, view.theme.current);
}

/** Where a stream a component opened stands. */
export type StreamStatus =
  | { readonly status: "idle" }
  | { readonly status: "opening" }
  | { readonly status: "open"; readonly stream: ViewStream }
  | { readonly status: "ended"; readonly end: StreamEnd }
  | { readonly status: "failed"; readonly error: HostError };

/**
 * Opens the stream of a grant while the component is shown, hands its pieces to
 * `onData`, and closes it when the component goes or the grant changes. A grant is
 * opened once: give `null` until the plugin's code has made one.
 */
export function useStream(grant: string | null, onData?: PieceListener): StreamStatus {
  const view = useView();
  const [status, setStatus] = useState<StreamStatus>({ status: "idle" });
  const latest = useRef(onData);
  latest.current = onData;
  useEffect(() => {
    if (grant === null) {
      setStatus({ status: "idle" });
      return undefined;
    }
    let gone = false;
    let open: ViewStream | null = null;
    setStatus({ status: "opening" });
    view.streams.open(grant).then(
      (stream) => {
        if (gone) {
          stream.close();
          return;
        }
        open = stream;
        stream.onData((piece) => latest.current?.(piece));
        setStatus({ status: "open", stream });
        void stream.ended.then((end) => {
          if (!gone) setStatus({ status: "ended", end });
        });
      },
      (error: HostError) => {
        if (!gone) setStatus({ status: "failed", error });
      },
    );
    return () => {
      gone = true;
      open?.close();
    };
  }, [view, grant]);
  return status;
}
