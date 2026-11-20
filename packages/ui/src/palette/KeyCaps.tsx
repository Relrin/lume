/**
 * Keys drawn as key caps, as the palettes show them: beside a command, in a footer that
 * says what the keys do.
 */

/** The modifiers a Mac writes as one sign each, with nothing between them. */
const MAC_SIGNS = /[⌘⇧⌥⌃]|[^⌘⇧⌥⌃]+/g;

/**
 * A key label, as `keyLabel` writes it, cut into chords and the keys of each:
 * `Ctrl+K Ctrl+T` is two chords of two keys. The key that types `+` stays whole.
 */
export function keyCaps(label: string): string[][] {
  return label
    .split(" ")
    .filter((chord) => chord !== "")
    .map((chord) =>
      chord.includes("+")
        ? chord.split(/\+(?=.)/).filter((key) => key !== "")
        : (chord.match(MAC_SIGNS) ?? [chord]),
    );
}

/** A key label as caps, a chord after another set apart by "then". */
export function KeyCaps({ label, className }: { label: string; className?: string }) {
  const chords = keyCaps(label);
  return (
    <span className={className ?? "key-caps"} aria-label={label}>
      {chords.map((chord, at) => (
        <span key={at} className="key-chord" aria-hidden="true">
          {at > 0 && <span className="key-then">then</span>}
          {chord.map((key, index) => (
            <kbd key={index}>{key}</kbd>
          ))}
        </span>
      ))}
    </span>
  );
}

/** One key cap, for a footer's legend. */
export function Cap({ children }: { children: string }) {
  return <kbd className="key-cap">{children}</kbd>;
}
