/**
 * The way between this page and the core: frames in both directions, and commands.
 *
 * Everything above this file talks to the {@link Carrier} interface, so the transport can
 * be exercised without a webview and the mechanism underneath can change without the
 * terminal noticing.
 */

import { Channel } from "@tauri-apps/api/core";
import { invoke } from "./invoke.ts";

import type { FrameLimits } from "./decoder.ts";

/** What the core says when a carrier opens. */
export interface CarrierOpened {
  /** Names this carrier in every later request. */
  readonly carrier: number;
  readonly limits: FrameLimits;
  /** How much output a stream may have in flight or waiting to be consumed. */
  readonly creditWindow: number;
  /** The backend new sessions start on. */
  readonly defaultBackend: string;
}

export interface Carrier {
  /**
   * Opens the carrier. From then on `receive` is called with the frames the core sends,
   * in order; each call's bytes are whole frames or a continuation of the previous call's.
   */
  open(receive: (frames: Uint8Array) => void): Promise<CarrierOpened>;

  /**
   * Sends frames to the core. The promise settles once the core has acted on them. Two
   * sends in flight may reach the core in either order, so a caller that cares waits.
   */
  send(frames: Uint8Array): Promise<void>;

  /** Calls one of the core's commands. A refusal rejects with what the core returned. */
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
}

/** The request header that names the carrier a raw request belongs to. */
const CARRIER_HEADER = "lume-carrier";

interface WireOpened {
  carrier: number;
  limits: {
    max_payload: number;
    max_control_message: number;
    max_hydrate: number;
    credit_window: number;
  };
  default_backend: string;
}

/**
 * A carrier inside the application's webview: frames arrive as raw bytes on a channel,
 * which keeps their order, and leave as the raw body of a command.
 */
export class TauriCarrier implements Carrier {
  #id: number | undefined;

  async open(receive: (frames: Uint8Array) => void): Promise<CarrierOpened> {
    const channel = new Channel<ArrayBuffer | number[]>((message) => {
      // Bytes normally arrive as a buffer. A webview that cannot fetch them gets a
      // list of numbers instead.
      receive(message instanceof ArrayBuffer ? new Uint8Array(message) : Uint8Array.from(message));
    });
    const opened = await invoke<WireOpened>("carrier_open", { channel });
    this.#id = opened.carrier;
    return {
      carrier: opened.carrier,
      limits: {
        maxPayload: opened.limits.max_payload,
        maxControlMessage: opened.limits.max_control_message,
        maxHydrate: opened.limits.max_hydrate,
      },
      creditWindow: opened.limits.credit_window,
      defaultBackend: opened.default_backend,
    };
  }

  async send(frames: Uint8Array): Promise<void> {
    if (this.#id === undefined) throw new Error("the carrier is not open");
    await invoke("carrier_send", frames, { headers: { [CARRIER_HEADER]: String(this.#id) } });
  }

  command<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
    return invoke<T>(name, args);
  }
}
