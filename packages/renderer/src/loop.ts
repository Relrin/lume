/**
 * The frame loop: one per window, drawing every terminal that has something new.
 *
 * Drawing is tied to the display's refresh so that a burst of output becomes one frame,
 * not one per chunk. The loop asks for an animation frame only when someone has said
 * there is something to draw, so an idle window schedules nothing at all, and it stops
 * while the window is hidden.
 */

/** Something the loop draws. */
export interface FrameView {
  /** Whether there is anything new to draw. Must be cheap: it is asked every frame. */
  needsFrame(): boolean;
  /** Draw. `now` is the time the frame started, in milliseconds. */
  frame(now: number): void;
}

/** What the loop needs from its surroundings. The defaults are the browser's. */
export interface FrameLoopEnvironment {
  requestFrame(callback: (now: number) => void): number;
  cancelFrame(handle: number): void;
  /** Whether the window is hidden, and how to hear when that changes. */
  isHidden(): boolean;
  onVisibilityChange(listener: () => void): () => void;
  /** Told when a view throws while drawing. */
  onError(error: unknown, view: FrameView): void;
}

function browserEnvironment(): FrameLoopEnvironment {
  return {
    requestFrame: (callback) => requestAnimationFrame(callback),
    cancelFrame: (handle) => cancelAnimationFrame(handle),
    isHidden: () => document.hidden,
    onVisibilityChange: (listener) => {
      document.addEventListener("visibilitychange", listener);
      return () => document.removeEventListener("visibilitychange", listener);
    },
    onError: (error) => console.error("a terminal view failed to draw", error),
  };
}

/** Draws views once per animation frame while any of them has something to draw. */
export class FrameLoop {
  readonly #env: FrameLoopEnvironment;
  readonly #views = new Set<FrameView>();
  readonly #stopListening: () => void;
  /** The pending animation frame, if one has been asked for. */
  #handle: number | undefined;
  #disposed = false;

  constructor(environment: Partial<FrameLoopEnvironment> = {}) {
    // The browser's services are only reached when called, so a loop given all of its
    // own can be built and run outside a browser.
    this.#env = { ...browserEnvironment(), ...environment };
    this.#stopListening = this.#env.onVisibilityChange(() => {
      if (!this.#env.isHidden()) this.requestFrame();
    });
  }

  /** Start drawing a view. */
  add(view: FrameView): void {
    this.#views.add(view);
    this.requestFrame();
  }

  /** Stop drawing a view. */
  remove(view: FrameView): void {
    this.#views.delete(view);
  }

  /**
   * Say that some view may have something to draw. Call it whenever a view's contents
   * change; asking twice before the frame arrives costs nothing.
   */
  requestFrame(): void {
    if (this.#disposed || this.#handle !== undefined) return;
    // A hidden window draws nothing. It is asked again when it is shown.
    if (this.#env.isHidden()) return;
    this.#handle = this.#env.requestFrame((now) => this.#tick(now));
  }

  /** Stop for good. */
  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    if (this.#handle !== undefined) this.#env.cancelFrame(this.#handle);
    this.#handle = undefined;
    this.#stopListening();
    this.#views.clear();
  }

  #tick(now: number): void {
    this.#handle = undefined;
    for (const view of [...this.#views]) {
      try {
        if (view.needsFrame()) view.frame(now);
      } catch (error) {
        // One terminal failing to draw must not stop the others.
        this.#env.onError(error, view);
      }
    }
    // A view that still has something to draw, such as one that is animating, keeps the
    // loop going. Otherwise it rests until the next request.
    for (const view of this.#views) {
      try {
        if (view.needsFrame()) {
          this.requestFrame();
          break;
        }
      } catch {
        // Already reported above if it throws consistently.
      }
    }
  }
}
