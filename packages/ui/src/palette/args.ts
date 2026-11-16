/**
 * What the palette has to ask before it can run a command.
 *
 * A command describes its arguments as a JSON Schema. The palette asks for the ones
 * the schema requires, one after another, and only for those it can ask for in a line
 * of text: words and whole numbers. An argument may say that its value is one of a
 * list the application keeps, such as the workspaces' names, which the palette then
 * offers to choose from.
 */

import type { CommandSpec } from "../core/types.ts";

/** The lists an argument's value may be chosen from. */
export type ChoiceSource = "workspaces" | "profiles";

const CHOICE_SOURCES: readonly string[] = ["workspaces", "profiles"] satisfies ChoiceSource[];

/** One thing on such a list: what the command is given, and what a person reads. */
export interface Choice {
  readonly value: string;
  readonly label: string;
}

/** The list a property's schema names, if it names one the palette can offer. */
function choicesOf(schema: Record<string, unknown>): ChoiceSource | null {
  const named = schema["x-lume-choices"];
  return typeof named === "string" && CHOICE_SOURCES.includes(named)
    ? (named as ChoiceSource)
    : null;
}

/** One thing the palette asks for. */
export interface ArgPrompt {
  /** The argument's name in what the command is given. */
  readonly name: string;
  readonly kind: "string" | "integer";
  /** What the field says while it is empty. */
  readonly placeholder: string;
  /** The list the value may be chosen from, if the argument names one. */
  readonly choices: ChoiceSource | null;
  /** The least a whole number may be, if the schema says. */
  readonly minimum: number | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The kind of value a property's schema asks for, of the kinds the palette can ask for. */
function kindOf(schema: Record<string, unknown>): ArgPrompt["kind"] | undefined {
  const type = schema["type"];
  const types = Array.isArray(type) ? type : [type];
  if (types.includes("string")) return "string";
  if (types.includes("integer")) return "integer";
  return undefined;
}

/**
 * What the palette asks for before running `command`: one prompt for each argument its
 * schema requires, in the order the schema requires them. A required argument that is
 * not a word or a whole number is left out; the command is run without it and says
 * what it lacks.
 */
export function promptsFor(command: Pick<CommandSpec, "args">): ArgPrompt[] {
  const schema = command.args;
  if (!isRecord(schema)) return [];
  const required = schema["required"];
  const properties = schema["properties"];
  if (!Array.isArray(required) || !isRecord(properties)) return [];
  const prompts: ArgPrompt[] = [];
  for (const name of required) {
    if (typeof name !== "string") continue;
    const property = properties[name];
    if (!isRecord(property)) continue;
    const kind = kindOf(property);
    if (!kind) continue;
    const description = property["description"];
    const minimum = property["minimum"];
    prompts.push({
      name,
      kind,
      placeholder: typeof description === "string" && description !== "" ? description : name,
      choices: choicesOf(property),
      minimum: typeof minimum === "number" ? minimum : null,
    });
  }
  return prompts;
}

/** What was typed, as the value a prompt asks for, or why it is not one. */
export type ArgValue =
  | { readonly ok: true; readonly value: string | number }
  | { readonly ok: false; readonly problem: string };

/** Reads what was typed in answer to a prompt. */
export function readArg(prompt: ArgPrompt, typed: string): ArgValue {
  const text = typed.trim();
  if (text === "") return { ok: false, problem: "Nothing was entered." };
  if (prompt.kind === "string") return { ok: true, value: text };
  if (!/^[+-]?\d+$/.test(text)) return { ok: false, problem: "A whole number is wanted." };
  const value = Number(text);
  if (!Number.isSafeInteger(value)) return { ok: false, problem: "The number is too large." };
  if (prompt.minimum !== null && value < prompt.minimum) {
    return { ok: false, problem: `The number must be at least ${prompt.minimum}.` };
  }
  return { ok: true, value };
}
