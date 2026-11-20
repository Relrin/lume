/**
 * What the command palette searches: everything, or only commands, the window's tabs or
 * the profiles a terminal is opened from, which the palette calls hosts.
 *
 * A scope is chosen with the chips under the field, with Tab and Shift+Tab, or by
 * starting what is typed with its sign: `>` for commands, `@` for tabs, `#` for hosts.
 * Backspace in an empty field goes back to everything.
 */

/** What the palette searches. */
export type Scope = "all" | "commands" | "tabs" | "hosts";

/** The scopes, in the order the chips show them. */
export const SCOPES: readonly { readonly value: Scope; readonly label: string }[] = [
  { value: "all", label: "All" },
  { value: "commands", label: "Commands" },
  { value: "tabs", label: "Tabs" },
  { value: "hosts", label: "Hosts" },
];

/** The sign that starts what is typed to search one scope. */
const SIGNS: Readonly<Record<string, Scope>> = { ">": "commands", "@": "tabs", "#": "hosts" };

/**
 * The scope what is typed asks for by its first sign, and the rest of what is typed, or
 * nothing when it starts with no sign.
 */
export function readSign(typed: string): { readonly scope: Scope; readonly rest: string } | null {
  const found = /^([>@#])\s?/.exec(typed);
  const scope = found ? SIGNS[found[1] ?? ""] : undefined;
  return found && scope ? { scope, rest: typed.slice(found[0].length) } : null;
}

/** The scope `by` steps from `scope`, round the chips. */
export function stepScope(scope: Scope, by: 1 | -1): Scope {
  const at = SCOPES.findIndex((choice) => choice.value === scope);
  const next = (at + by + SCOPES.length) % SCOPES.length;
  return SCOPES[next]?.value ?? "all";
}

/** How many rows of each kind are listed under everything with something typed. */
export const PER_KIND_MAX = 8;
