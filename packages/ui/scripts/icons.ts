/**
 * Writes `src/icons/glyphs.generated.ts` and `src/icons/NOTICE.md` from the two icon
 * packages the page draws with: Lucide for its line drawings and Simple Icons for the
 * marks of well-known programs.
 *
 * Only the glyphs named below are copied, so the page carries neither package. A name
 * the installed version does not have stops the script and says which. A test writes
 * the module again in memory and fails when the file kept here differs.
 *
 * Run with `pnpm --filter @lume/ui icons` after changing a list or a package's version.
 */

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";

/**
 * The line drawings the page uses, by their Lucide names. A plugin may name any of them
 * for the icon of a thing it adds, so the list holds some the page itself does not use.
 */
export const LINE_NAMES: readonly string[] = [
  "activity",
  "arrow-down",
  "arrow-right",
  "arrow-up",
  "battery",
  "bell",
  "bell-off",
  "book-open",
  "bookmark",
  "bot",
  "box",
  "bug",
  "chart-bar",
  "chart-line",
  "check",
  "chevron-down",
  "chevron-first",
  "chevron-last",
  "chevron-left",
  "chevron-right",
  "circle",
  "circle-exclamation-point",
  "circle-check",
  "clipboard",
  "clock",
  "cloud",
  "columns-2",
  "command",
  "contrast",
  "copy",
  "cpu",
  "database",
  "download",
  "eye",
  "file",
  "file-text",
  "flame",
  "flask-conical",
  "fold-horizontal",
  "folder",
  "folder-input",
  "folder-open",
  "gauge",
  "git-branch",
  "globe",
  "hard-drive",
  "heart",
  "info",
  "key-round",
  "keyboard",
  "layers",
  "list",
  "lock",
  "maximize-2",
  "memory-stick",
  "menu",
  "microchip",
  "minus",
  "monitor",
  "network",
  "palette",
  "panel-bottom",
  "panel-left",
  "panel-right",
  "panel-top",
  "pin",
  "pipette",
  "pause",
  "pencil",
  "play",
  "plus",
  "puzzle",
  "refresh-cw",
  "rocket",
  "rotate-ccw",
  "rotate-cw",
  "rows-2",
  "search",
  "server",
  "settings",
  "sparkles",
  "square",
  "square-dashed-text",
  "square-plus",
  "square-x",
  "star",
  "terminal",
  "thermometer",
  "triangle-exclamation-point",
  "unfold-horizontal",
  "ungroup",
  "upload",
  "wifi",
  "wrench",
  "x",
  "zap",
];

/**
 * The marks of programs the page knows, by their Simple Icons slugs: programs that run
 * in a terminal, and the Linux distributions WSL runs. Programs that only open windows
 * of their own, such as terminal emulators and most IDEs, are left out.
 */
export const BRAND_SLUGS: readonly string[] = [
  // Shells and multiplexers.
  "fishshell",
  "gnubash",
  "nushell",
  "tmux",
  "zsh",
  // Editors, and editors started from a terminal to edit one file.
  "gnuemacs",
  "helix",
  "lapce",
  "neovim",
  "obsidian",
  "sublimetext",
  "vim",
  "vscodium",
  "zedindustries",
  // Version control and the command lines of forges.
  "fossilscm",
  "git",
  "gitea",
  "github",
  "gitlab",
  "forgejo",
  "mercurial",
  "subversion",
  // Coding agents and the command lines of AI providers.
  "claude",
  "cline",
  "coderabbit",
  "cursor",
  "githubcopilot",
  "googlegemini",
  "huggingface",
  "lmstudio",
  "mistralai",
  "ollama",
  "opencode",
  "qwen",
  // Databases and their clients.
  "apachecassandra",
  "clickhouse",
  "cockroachlabs",
  "duckdb",
  "elasticsearch",
  "firebase",
  "googlebigquery",
  "influxdb",
  "mariadb",
  "mongodb",
  "mysql",
  "neo4j",
  "planetscale",
  "postgresql",
  "prisma",
  "redis",
  "snowflake",
  "sqlite",
  "supabase",
  "surrealdb",
  "turso",
  // Languages, their runtimes and their build tools.
  "bazel",
  "bun",
  "clojure",
  "cmake",
  "crystal",
  "dart",
  "deno",
  "dotnet",
  "elixir",
  "erlang",
  "flutter",
  "gleam",
  "go",
  "gradle",
  "haskell",
  "julia",
  "jupyter",
  "kotlin",
  "lua",
  "nim",
  "nodedotjs",
  "ocaml",
  "openjdk",
  "perl",
  "php",
  "python",
  "r",
  "racket",
  "ruby",
  "rubyonrails",
  "rust",
  "scala",
  "swift",
  "typescript",
  "vite",
  "zig",
  // Package managers.
  "anaconda",
  "chocolatey",
  "homebrew",
  "nixos",
  "npm",
  "pnpm",
  "poetry",
  "uv",
  "yarn",
  // Containers, clouds and infrastructure.
  "ansible",
  "argo",
  "cloudflare",
  "consul",
  "digitalocean",
  "docker",
  "flydotio",
  "googlecloud",
  "helm",
  "k3s",
  "kubernetes",
  "netlify",
  "nomad",
  "opentofu",
  "packer",
  "pulumi",
  "railway",
  "terraform",
  "vagrant",
  "vault",
  "vercel",
  // Network, security and media tools.
  "1password",
  "bitwarden",
  "caddy",
  "curl",
  "ffmpeg",
  "gnuprivacyguard",
  "htop",
  "httpie",
  "nginx",
  "tailscale",
  "wireguard",
  "wireshark",
  // Linux distributions, as WSL names them.
  "almalinux",
  "alpinelinux",
  "archlinux",
  "centos",
  "debian",
  "fedora",
  "gentoo",
  "kalilinux",
  "linuxmint",
  "manjaro",
  "opensuse",
  "redhat",
  "rockylinux",
  "slackware",
  "ubuntu",
  "voidlinux",
];

/**
 * How long a mark's path may be, in characters. A longer one is drawn in more detail
 * than a tile ten pixels high can show, and costs the page more to carry.
 */
export const BRAND_PATH_MAX = 5200;

/** The files the script writes, relative to the package. */
export const OUTPUTS = {
  glyphs: "src/icons/glyphs.generated.ts",
  notice: "src/icons/NOTICE.md",
} as const;

type Node = readonly [string, Readonly<Record<string, string>>];

interface Brand {
  readonly title: string;
  readonly slug: string;
  readonly hex: string;
}

const PACKAGE = fileURLToPath(new URL("..", import.meta.url));

function read(path: string): string {
  return readFileSync(`${PACKAGE}/${path}`, "utf8");
}

function version(name: string): string {
  return (JSON.parse(read(`node_modules/${name}/package.json`)) as { version: string }).version;
}

/** The relative luminance of a colour written as six hex digits. */
function luminance(hex: string): number {
  const channel = (at: number) => {
    const value = parseInt(hex.slice(at, at + 2), 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4);
}

/** The one path of a Simple Icons mark. */
function brandPath(slug: string): string {
  const svg = read(`node_modules/simple-icons/icons/${slug}.svg`);
  const paths = [...svg.matchAll(/<path d="([^"]+)"\s*\/>/g)];
  const only = paths[0]?.[1];
  if (paths.length !== 1 || only === undefined) {
    throw new Error(`simple-icons: ${slug} is not drawn as one path`);
  }
  if (only.length > BRAND_PATH_MAX) {
    throw new Error(
      `simple-icons: ${slug} is drawn in ${only.length} characters, more than ${BRAND_PATH_MAX}`,
    );
  }
  return only;
}

function missing(kind: string, names: readonly string[]): never {
  throw new Error(`${kind} has no ${names.length === 1 ? "glyph" : "glyphs"} ${names.join(", ")}`);
}

/** What the script writes, by path, without writing it. */
export function generate(): Record<string, string> {
  const lucide = version("lucide-static");
  const simple = version("simple-icons");
  const nodes = JSON.parse(read("node_modules/lucide-static/icon-nodes.json")) as Record<
    string,
    readonly Node[]
  >;
  const absentLines = LINE_NAMES.filter((name) => nodes[name] === undefined);
  if (absentLines.length > 0) missing(`lucide-static ${lucide}`, absentLines);

  const data = JSON.parse(
    read("node_modules/simple-icons/data/simple-icons.json"),
  ) as readonly Brand[];
  const brands = new Map(data.map((brand) => [brand.slug, brand] as const));
  const absentBrands = BRAND_SLUGS.filter((slug) => !brands.has(slug));
  if (absentBrands.length > 0) missing(`simple-icons ${simple}`, absentBrands);

  const lines = [...LINE_NAMES].sort();
  const slugs = [...BRAND_SLUGS].sort();
  const out: string[] = [
    `// Written by \`pnpm --filter @lume/ui icons\` from lucide-static ${lucide} and`,
    `// simple-icons ${simple}. Do not edit: change the lists in scripts/icons.ts instead.`,
    `// Where the drawings come from and on what terms is in NOTICE.md beside this file.`,
    ``,
    `/** One element of a line drawing: its tag and its attributes. */`,
    `export type GlyphNode = readonly [tag: string, attributes: Readonly<Record<string, string>>];`,
    ``,
    `/** The mark of a program, drawn as one filled path on a 24 by 24 square. */`,
    `export interface BrandGlyph {`,
    `  /** What the brand is called. */`,
    `  readonly title: string;`,
    `  /** The brand's own colour. */`,
    `  readonly colour: string;`,
    `  /** The colour the mark is drawn in on the brand's colour: white, or near black on a light one. */`,
    `  readonly ink: string;`,
    `  /** Whether the colour is so pale that a tile of it needs an edge. */`,
    `  readonly pale: boolean;`,
    `  readonly path: string;`,
    `}`,
    ``,
    `export type LineGlyphName =`,
    ...lines.map(
      (name, index) => `  | ${JSON.stringify(name)}${index === lines.length - 1 ? ";" : ""}`,
    ),
    ``,
    `export type BrandGlyphName =`,
    ...slugs.map(
      (slug, index) => `  | ${JSON.stringify(slug)}${index === slugs.length - 1 ? ";" : ""}`,
    ),
    ``,
    `/** Line drawings on a 24 by 24 square, stroked in the colour of the text around them. */`,
    `export const LINE_GLYPHS: Readonly<Record<LineGlyphName, readonly GlyphNode[]>> = {`,
  ];
  for (const name of lines) {
    out.push(`  ${JSON.stringify(name)}: [`);
    for (const [tag, attributes] of nodes[name] ?? []) {
      out.push(`    [${JSON.stringify(tag)}, ${JSON.stringify(attributes)}],`);
    }
    out.push(`  ],`);
  }
  out.push(`};`, ``);
  out.push(`/** The marks of programs. */`);
  out.push(`export const BRAND_GLYPHS: Readonly<Record<BrandGlyphName, BrandGlyph>> = {`);
  for (const slug of slugs) {
    const brand = brands.get(slug);
    if (!brand) continue;
    const light = luminance(brand.hex);
    out.push(`  ${JSON.stringify(slug)}: {`);
    out.push(`    title: ${JSON.stringify(brand.title)},`);
    out.push(`    colour: "#${brand.hex.toLowerCase()}",`);
    out.push(`    ink: ${JSON.stringify(light > 0.45 ? "#1a1a1a" : "#ffffff")},`);
    out.push(`    pale: ${String(light > 0.8)},`);
    out.push(`    path: ${JSON.stringify(brandPath(slug))},`);
    out.push(`  },`);
  }
  out.push(`};`, ``);

  const notice = [
    `# Icons`,
    ``,
    `\`glyphs.generated.ts\` holds drawings copied from two packages.`,
    ``,
    `## Lucide`,
    ``,
    `The line drawings are from lucide-static ${lucide}, <https://lucide.dev>.`,
    ``,
    "```text",
    read("node_modules/lucide-static/LICENSE").trim(),
    "```",
    ``,
    `## Simple Icons`,
    ``,
    `The marks of programs are from simple-icons ${simple}, <https://simpleicons.org>,`,
    `whose drawings are given to the public domain under CC0 1.0 Universal.`,
    ``,
    `Each mark is a trademark of its owner. It is shown only to say which program runs in`,
    `a pane, and showing it implies no endorsement by that owner. Copied here:`,
    ``,
    ...slugs.map((slug) => `- ${brands.get(slug)?.title ?? slug}`),
    ``,
  ];
  return { [OUTPUTS.glyphs]: out.join("\n"), [OUTPUTS.notice]: notice.join("\n") };
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  for (const [path, text] of Object.entries(generate())) {
    writeFileSync(`${PACKAGE}/${path}`, text);
    console.log(`icons: wrote ${path}`);
  }
}
