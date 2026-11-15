/**
 * Small pieces every measurement uses: what a set of samples says, and waiting for the
 * next frame.
 */

/** What a set of time samples says, in milliseconds. */
export interface Timing {
  readonly samples: number;
  readonly p50: number;
  readonly p95: number;
  readonly p99: number;
  readonly max: number;
}

function percentile(sorted: readonly number[], p: number): number {
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))] ?? 0;
}

/** Thousandths of a millisecond are as fine as the clock is worth trusting. */
function round(ms: number): number {
  return Math.round(ms * 1000) / 1000;
}

/** Summarises samples in milliseconds. */
export function timing(samples: readonly number[]): Timing {
  const sorted = [...samples].sort((a, b) => a - b);
  return {
    samples: sorted.length,
    p50: round(percentile(sorted, 0.5)),
    p95: round(percentile(sorted, 0.95)),
    p99: round(percentile(sorted, 0.99)),
    max: round(sorted[sorted.length - 1] ?? 0),
  };
}

/**
 * The next animation frame. A hidden page gets none, so after a short wait it goes on
 * anyway: a measurement must also finish where nobody is looking at it.
 */
export function nextFrame(): Promise<void> {
  return new Promise((resolve) => {
    const fallback = setTimeout(resolve, 50);
    requestAnimationFrame(() => {
      clearTimeout(fallback);
      resolve();
    });
  });
}

/** Resolves after `ms` milliseconds. */
export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
