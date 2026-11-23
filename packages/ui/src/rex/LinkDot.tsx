/**
 * Whether a host can be used, beside its name: a dot, or a ring that turns while a
 * connection to it is being made. The turning ring stands for words, and says them to a
 * screen reader; what the dot means is said by what stands beside it.
 */

import type { RexLink } from "./model.ts";

/** The dot. `className` is how it is drawn where it stands. */
export function LinkDot({ link, className }: { link: RexLink; className: string }) {
  if (link !== "busy") return <span className={className} data-link={link} aria-hidden="true" />;
  return <span className={className} data-link="busy" role="img" aria-label="Connecting" />;
}
