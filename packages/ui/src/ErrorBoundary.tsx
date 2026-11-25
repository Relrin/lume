/**
 * Keeps a part of the page that cannot be drawn from taking the rest of it along.
 *
 * React takes a whole page away when something in it throws while it is drawn. A boundary
 * takes away only what it is around, and draws something else in its place: what that is,
 * and when the part is drawn again, is up to whoever puts the boundary there. What was
 * thrown is written to the console, by React, with where in the page it was.
 */

import { Component, type ReactNode } from "react";

import { plain } from "./text/plain.ts";

/** The longest account of a failure that is shown, in bytes of UTF-8. */
const MESSAGE_MAX_BYTES = 512;

/** A part that could not be drawn. */
export interface Failure {
  /** What was thrown, in words that may be shown. */
  readonly message: string;
  /** How many times the part has failed by now. */
  readonly times: number;
  /** Draws the part again. */
  readonly again: () => void;
}

/** What was thrown, in words that may be shown. It may be anything at all. */
export function failureMessage(thrown: unknown): string {
  try {
    const said = thrown instanceof Error ? thrown.message : String(thrown);
    return plain(said, MESSAGE_MAX_BYTES) || "Nothing more is known.";
  } catch {
    // Not everything that can be thrown can be put in words.
    return "Nothing more is known.";
  }
}

interface Props {
  readonly children: ReactNode;
  /** What is drawn in the part's place. */
  readonly fallback: (failure: Failure) => ReactNode;
  /** Told each time the part fails, once the page is without it. */
  readonly failed?: (failure: Failure) => void;
}

interface State {
  readonly message: string | null;
}

export class ErrorBoundary extends Component<Props, State> {
  override state: State = { message: null };
  #times = 0;

  static getDerivedStateFromError(thrown: unknown): State {
    return { message: failureMessage(thrown) };
  }

  override componentDidCatch(thrown: unknown): void {
    this.#times += 1;
    this.props.failed?.(this.#failure(failureMessage(thrown)));
  }

  readonly #again = (): void => this.setState({ message: null });

  #failure(message: string): Failure {
    return { message, times: this.#times, again: this.#again };
  }

  override render(): ReactNode {
    const { message } = this.state;
    return message === null ? this.props.children : this.props.fallback(this.#failure(message));
  }
}
