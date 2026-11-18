/**
 * An attachment with a test at the other end instead of a session: it records what the
 * terminal's side does and lets the test play the session's part.
 */

import type { Attachment, StreamFrame } from "../src/transport/client.ts";
import type {
  BackendCaps,
  CellPx,
  ClientFacts,
  Responder,
  TermSize,
} from "../src/transport/types.ts";

const utf8 = (text: string) => new TextEncoder().encode(text);
const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

/** An attachment with a test at the other end instead of a session. */
export class FakeAttachment implements Attachment {
  readonly size: TermSize = { cols: 80, rows: 24 };
  readonly resizePullsScrollback = false;
  readonly hydrate = "none" as const;
  readonly caps: BackendCaps;
  /** Everything the terminal's side did, in order. */
  readonly calls: string[] = [];
  readonly inputs: string[] = [];
  consumedBytes = 0;
  /** Sends a frame to the terminal, as the session would. */
  deliver: (frame: StreamFrame) => void = () => {
    throw new Error("the attachment has not been started");
  };

  constructor(responder: Responder = "client", hydrates = false, multiClient = false) {
    this.caps = {
      persistent: false,
      multiClient,
      serverLayout: false,
      responder,
      hydrate: { snapshot: hydrates, vtReplay: hydrates },
    };
  }

  rehydrate(): void {
    this.calls.push("rehydrate");
  }
  release(): void {
    this.calls.push("release");
  }
  hydrated(): Promise<undefined> {
    return Promise.resolve(undefined);
  }
  start(receive: (frame: StreamFrame) => void): void {
    this.calls.push("start");
    this.deliver = receive;
  }
  input(bytes: Uint8Array): void {
    this.calls.push("input");
    this.inputs.push(text(bytes));
  }
  resize(size: TermSize, cellPx: CellPx): void {
    this.calls.push(`resize ${size.cols}x${size.rows} ${cellPx.w}x${cellPx.h}`);
  }
  facts(facts: ClientFacts): void {
    this.calls.push(`facts focused=${facts.focused}`);
  }
  consumed(bytes: number): void {
    this.consumedBytes += bytes;
  }
  detach(): void {
    this.calls.push("detach");
  }

  /** Delivers program output to the terminal: text, or bytes as they are. */
  data(output: string | Uint8Array): void {
    const bytes = typeof output === "string" ? utf8(output) : output;
    this.deliver({ stream: 1, kind: "data", bytes });
  }
}
