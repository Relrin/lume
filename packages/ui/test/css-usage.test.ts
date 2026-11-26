/**
 * Every class the stylesheets style is one the code uses, and every class the code
 * names in a `className` is one a stylesheet styles.
 *
 * The stylesheets are read from `src/styles.css` and the files it brings in. The code
 * is every `.ts` and `.tsx` under `src`: its strings, and the fixed parts of its
 * template strings, split at white space. A class counts as used when one of those
 * words is its name, which covers `classList.add("pressed")`, `.className = "x"` and
 * `anchored ${className}` alike, and when a selector string such as `".tab .close"`
 * names it. A word that runs into a `${…}` is the start or the end of a name made as
 * the code runs: every class that starts or ends that way counts as used.
 *
 * What is styled but named only somewhere else, such as by a library, is listed with
 * its reason in `css-usage.allow.json`.
 */
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import ts from "typescript";
import { describe, expect, it } from "vitest";

const UI = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SRC = join(UI, "src");
const shown = (file: string) => relative(UI, file).replace(/\\/g, "/");

interface Allowed {
  /** Classes styled that the code does not name, each with why. */
  styled: Record<string, string>;
  /** Classes named in a `className` that no stylesheet styles, each with why. */
  named: Record<string, string>;
}

const allowed = JSON.parse(
  readFileSync(join(UI, "test", "css-usage.allow.json"), "utf8"),
) as Allowed;

/* The stylesheets. */

/** The text of a stylesheet without its comments, strings and `url(…)`s. */
function bare(css: string): string {
  return css
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/url\(\s*(?:"[^"]*"|'[^']*'|[^)]*)\s*\)/g, "url()")
    .replace(/"(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'/g, '""');
}

/** `styles.css` and every stylesheet it brings in from this package, in order. */
function stylesheets(entry: string): Map<string, string> {
  const found = new Map<string, string>();
  const visit = (file: string) => {
    if (found.has(file)) return;
    const text = readFileSync(file, "utf8");
    found.set(file, text);
    const stripped = text.replace(/\/\*[\s\S]*?\*\//g, " ");
    for (const [, path] of stripped.matchAll(/@import\s+["'](\.{1,2}\/[^"']+)["']/g)) {
      visit(resolve(dirname(file), path!));
    }
  };
  visit(entry);
  return found;
}

/** The classes a stylesheet's selectors name. At-rules' own preludes are not selectors. */
function styledClasses(css: string): Set<string> {
  const classes = new Set<string>();
  const text = bare(css);
  let start = 0;
  for (let at = 0; at < text.length; at++) {
    const c = text[at];
    if (c === ";" || c === "}") start = at + 1;
    else if (c === "{") {
      const prelude = text.slice(start, at).trim();
      start = at + 1;
      if (prelude.startsWith("@")) continue;
      for (const [, name] of prelude.matchAll(/\.(-?[_a-zA-Z][\w-]*)/g)) classes.add(name!);
    }
  }
  return classes;
}

/* The code. */

function sources(dir: string): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...sources(path));
    else if (/\.tsx?$/.test(entry.name) && !/\.generated\.ts$/.test(entry.name)) files.push(path);
  }
  return files;
}

/** The words of a piece of text, and those that run into a `${…}` at either end. */
interface Words {
  whole: string[];
  starts: string[];
  ends: string[];
}

/** Splits a fixed piece of text. `before`/`after`: whether a `${…}` touches it there. */
function split(text: string, before = false, after = false): Words {
  const words = text.split(/\s+/);
  const whole: string[] = [];
  const starts: string[] = [];
  const ends: string[] = [];
  words.forEach((word, index) => {
    if (!word) return;
    const first = index === 0 && before;
    const last = index === words.length - 1 && after;
    if (first && last) return;
    if (last) starts.push(word);
    else if (first) ends.push(word);
    else whole.push(word);
  });
  return { whole, starts, ends };
}

function templateWords(node: ts.TemplateExpression): Words[] {
  const pieces = [split(node.head.text, false, true)];
  for (const span of node.templateSpans) {
    pieces.push(split(span.literal.text, true, ts.isTemplateMiddle(span.literal)));
  }
  return pieces;
}

/** The words a `className`'s value can give: the strings it can be, not those it compares. */
function classNameWords(node: ts.Node, into: Words[]): void {
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
    into.push(split(node.text));
  } else if (ts.isTemplateExpression(node)) {
    into.push(...templateWords(node));
    for (const span of node.templateSpans) classNameWords(span.expression, into);
  } else if (ts.isJsxExpression(node) || ts.isParenthesizedExpression(node)) {
    if (node.expression) classNameWords(node.expression, into);
  } else if (ts.isConditionalExpression(node)) {
    classNameWords(node.whenTrue, into);
    classNameWords(node.whenFalse, into);
  } else if (ts.isBinaryExpression(node)) {
    const op = node.operatorToken.kind;
    if (op === ts.SyntaxKind.BarBarToken || op === ts.SyntaxKind.QuestionQuestionToken) {
      classNameWords(node.left, into);
    }
    if (
      op === ts.SyntaxKind.BarBarToken ||
      op === ts.SyntaxKind.QuestionQuestionToken ||
      op === ts.SyntaxKind.AmpersandAmpersandToken ||
      op === ts.SyntaxKind.PlusToken
    ) {
      classNameWords(node.right, into);
    }
    if (op === ts.SyntaxKind.PlusToken) classNameWords(node.left, into);
  } else if (ts.isArrayLiteralExpression(node)) {
    for (const element of node.elements) classNameWords(element, into);
  } else if (ts.isCallExpression(node)) {
    // `[…].join(" ")` and helpers that join their arguments.
    if (ts.isPropertyAccessExpression(node.expression)) {
      classNameWords(node.expression.expression, into);
    } else {
      for (const arg of node.arguments) classNameWords(arg, into);
    }
  }
}

interface Code {
  /** Every word of every string in the code. */
  words: Set<string>;
  /** Fixed parts that run into a `${…}`: the start of a name, or its end. */
  starts: Set<string>;
  ends: Set<string>;
  /** Each whole word a `className` gives, with the files it is given in. */
  named: Map<string, Set<string>>;
}

function readCode(files: string[]): Code {
  const code: Code = { words: new Set(), starts: new Set(), ends: new Set(), named: new Map() };
  const keep = (pieces: Words[]) => {
    for (const { whole, starts, ends } of pieces) {
      for (const word of whole) {
        code.words.add(word);
        // A selector string names classes with a dot before each.
        for (const [, name] of word.matchAll(/\.(-?[_a-zA-Z][\w-]*)/g)) code.words.add(name!);
      }
      // Only what reads as part of a name: `tab-${kind}`, `${part}-list`. A letter or a
      // `#` before a `${…}` is some other text, and would pass any class.
      for (const word of starts) {
        const start = word.replace(/^.*\./, "");
        if (/^-?[_a-zA-Z][\w-]*[-_]$/.test(start)) code.starts.add(start);
      }
      for (const word of ends) if (/^[-_][\w-]*[a-zA-Z0-9]$/.test(word)) code.ends.add(word);
    }
  };
  for (const file of files) {
    const source = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.Latest);
    const visit = (node: ts.Node): void => {
      if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
        keep([split(node.text)]);
      } else if (ts.isTemplateExpression(node)) {
        keep(templateWords(node));
      } else if (
        ts.isJsxAttribute(node) &&
        ts.isIdentifier(node.name) &&
        node.name.text === "className" &&
        node.initializer
      ) {
        const pieces: Words[] = [];
        classNameWords(node.initializer, pieces);
        for (const { whole } of pieces) {
          for (const word of whole) {
            const where = code.named.get(word) ?? new Set();
            where.add(shown(file));
            code.named.set(word, where);
          }
        }
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
  }
  return code;
}

/* The checks. */

const sheets = stylesheets(join(SRC, "styles.css"));
const styled = new Map<string, Set<string>>();
for (const [file, text] of sheets) {
  for (const name of styledClasses(text)) {
    const where = styled.get(name) ?? new Set();
    where.add(shown(file));
    styled.set(name, where);
  }
}
const code = readCode(sources(SRC));

const report = (found: Map<string, Set<string>>) =>
  [...found].map(([name, files]) => `${name}  (${[...files].sort().join(", ")})`).sort();

describe("the classes the stylesheets style", () => {
  it("are read from styles.css and the files it brings in", () => {
    expect(sheets.size).toBeGreaterThan(1);
    expect(styled.size).toBeGreaterThan(100);
  });

  it("are each used by the code", () => {
    const unused = new Map<string, Set<string>>();
    for (const [name, files] of styled) {
      if (name in allowed.styled || code.words.has(name)) continue;
      if ([...code.starts].some((start) => name.startsWith(start))) continue;
      if ([...code.ends].some((end) => name.endsWith(end))) continue;
      unused.set(name, files);
    }
    // Take the rule out, or list the class in css-usage.allow.json with why it stays.
    expect(report(unused), "classes styled that no code uses").toEqual([]);
  });

  it("are listed as allowed only while they are still styled and still unused", () => {
    const stale = Object.keys(allowed.styled).filter(
      (name) => !styled.has(name) || code.words.has(name),
    );
    expect(stale, "entries of css-usage.allow.json's `styled` to take out").toEqual([]);
  });
});

describe("the classes the code names", () => {
  it("are each styled by a stylesheet", () => {
    const unstyled = new Map<string, Set<string>>();
    for (const [name, files] of code.named) {
      if (!styled.has(name) && !(name in allowed.named)) unstyled.set(name, files);
    }
    // Style the class, take it out of the className, or list it in css-usage.allow.json.
    expect(report(unstyled), "classes named in a className that nothing styles").toEqual([]);
  });

  it("are listed as allowed only while nothing styles them", () => {
    const stale = Object.keys(allowed.named).filter(
      (name) => styled.has(name) || !code.named.has(name),
    );
    expect(stale, "entries of css-usage.allow.json's `named` to take out").toEqual([]);
  });
});
