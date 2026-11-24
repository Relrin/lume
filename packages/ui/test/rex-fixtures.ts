import type { RexHost, RexHosts, RexHostState } from "../src/core/types.ts";

export const UP: RexHostState = {
  kind: "connected",
  version: "0.1.0",
  platform: "linux_arm64",
  api_revision: 19,
};

export function rexHost(id: string, label: string, extra: Partial<RexHost> = {}): RexHost {
  return {
    id: `rex:${id}`,
    label,
    address: id,
    local: false,
    written: true,
    state: { kind: "idle" },
    sessions: [],
    on_show: null,
    latency_ms: null,
    ...extra,
  };
}

export function rexFailed(code: string, error: string): RexHostState {
  return { kind: "failed", code, error };
}

export const DEVBOX = rexHost("ssh://devbox", "devbox", {
  state: UP,
  latency_ms: 12,
  sessions: [
    { id: "session:1", label: "acme" },
    { id: "session:2", label: "ml-train" },
  ],
});
export const NAS = rexHost("https://nas", "home-nas");

export function rexList(...hosts: RexHost[]): RexHosts {
  return { revision: 1, local: true, hosts };
}
