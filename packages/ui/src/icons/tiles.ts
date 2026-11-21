/**
 * The tiles a pane's header, its tab and its marker show for the program it runs: a
 * small rounded square of colour with a drawing on it.
 *
 * A kind of program gets a line drawing on a colour of the terminal's palette, so it
 * suits the theme; Git and its tools get Git's mark. A well-known program gets its own
 * mark on its own colour. A program
 * known to neither gets a prompt on a palette colour picked by its name, the same one
 * each time.
 */

import type { IconRef } from "../core/types.ts";
import { BRAND_GLYPHS, type BrandGlyphName, type LineGlyphName } from "./glyphs.generated.ts";

/** A colour of the terminal's palette, as the page's tokens hold it. */
function palette(index: number): string {
  return `var(--lume-terminal-palette-${index})`;
}

/**
 * The ink drawn on a colour of the palette: the terminal's background. A theme makes
 * its colours readable on that background, and the same pair reads as well the other
 * way round: light on a dark theme's bright colours would not.
 */
const INK = "var(--lume-terminal-bg)";

/** What a tile is drawn with. */
export interface Tile {
  readonly glyph:
    | { readonly kind: "line"; readonly name: LineGlyphName }
    | { readonly kind: "brand"; readonly name: BrandGlyphName };
  readonly background: string;
  readonly ink: string;
  /** Whether the tile is pale enough to need an edge of its own. */
  readonly edge: boolean;
}

/** The kinds of program, each with a line drawing. */
export const CATEGORY_TILES = [
  "terminal",
  "editor",
  "git",
  "monitor",
  "agent",
  "remote",
  "container",
  "files",
] as const;

export type CategoryTile = (typeof CATEGORY_TILES)[number];

const CATEGORIES: Readonly<Record<CategoryTile, Tile>> = {
  // A shell is a prompt in green on black, as a terminal shows one.
  terminal: line("terminal", 0, palette(10)),
  editor: line("file-text", 4, INK),
  // Git and the tools that work on it: Git's own mark.
  git: brand("git"),
  monitor: line("activity", 2, INK),
  agent: line("sparkles", 5, INK),
  remote: line("globe", 6, INK),
  container: line("box", 12, INK),
  files: line("folder", 3, INK),
};

function line(name: LineGlyphName, colour: number, ink: string): Tile {
  return { glyph: { kind: "line", name }, background: palette(colour), ink, edge: false };
}

function brand(name: BrandGlyphName): Tile {
  const mark = BRAND_GLYPHS[name];
  return {
    glyph: { kind: "brand", name },
    background: mark.colour,
    ink: mark.ink,
    edge: mark.pale,
  };
}

/**
 * The tiles of well-known programs, each named for the program and drawn with its
 * mark. The configuration names them by these names.
 */
export const BRAND_TILES = {
  // Shells and multiplexers.
  bash: "gnubash",
  fish: "fishshell",
  nushell: "nushell",
  tmux: "tmux",
  zsh: "zsh",
  // Editors.
  emacs: "gnuemacs",
  helix: "helix",
  lapce: "lapce",
  neovim: "neovim",
  obsidian: "obsidian",
  sublime: "sublimetext",
  vim: "vim",
  vscodium: "vscodium",
  zed: "zedindustries",
  // Version control and forges.
  forgejo: "forgejo",
  fossil: "fossilscm",
  gitea: "gitea",
  github: "github",
  gitlab: "gitlab",
  mercurial: "mercurial",
  subversion: "subversion",
  // Coding agents and AI providers.
  claude: "claude",
  cline: "cline",
  coderabbit: "coderabbit",
  copilot: "githubcopilot",
  cursor: "cursor",
  gemini: "googlegemini",
  huggingface: "huggingface",
  lmstudio: "lmstudio",
  mistral: "mistralai",
  ollama: "ollama",
  opencode: "opencode",
  qwen: "qwen",
  // Databases.
  bigquery: "googlebigquery",
  cassandra: "apachecassandra",
  clickhouse: "clickhouse",
  cockroachdb: "cockroachlabs",
  duckdb: "duckdb",
  elasticsearch: "elasticsearch",
  firebase: "firebase",
  influxdb: "influxdb",
  mariadb: "mariadb",
  mongodb: "mongodb",
  mysql: "mysql",
  neo4j: "neo4j",
  planetscale: "planetscale",
  postgres: "postgresql",
  prisma: "prisma",
  redis: "redis",
  snowflake: "snowflake",
  sqlite: "sqlite",
  supabase: "supabase",
  surrealdb: "surrealdb",
  turso: "turso",
  // Languages, runtimes and build tools.
  bazel: "bazel",
  bun: "bun",
  clojure: "clojure",
  cmake: "cmake",
  crystal: "crystal",
  dart: "dart",
  deno: "deno",
  dotnet: "dotnet",
  elixir: "elixir",
  erlang: "erlang",
  flutter: "flutter",
  gleam: "gleam",
  go: "go",
  gradle: "gradle",
  haskell: "haskell",
  java: "openjdk",
  julia: "julia",
  jupyter: "jupyter",
  kotlin: "kotlin",
  lua: "lua",
  nim: "nim",
  node: "nodedotjs",
  ocaml: "ocaml",
  perl: "perl",
  php: "php",
  python: "python",
  r: "r",
  racket: "racket",
  rails: "rubyonrails",
  ruby: "ruby",
  rust: "rust",
  scala: "scala",
  swift: "swift",
  typescript: "typescript",
  vite: "vite",
  zig: "zig",
  // Package managers.
  chocolatey: "chocolatey",
  conda: "anaconda",
  homebrew: "homebrew",
  nix: "nixos",
  npm: "npm",
  pnpm: "pnpm",
  poetry: "poetry",
  uv: "uv",
  yarn: "yarn",
  // Containers, clouds and infrastructure.
  ansible: "ansible",
  argo: "argo",
  cloudflare: "cloudflare",
  consul: "consul",
  digitalocean: "digitalocean",
  docker: "docker",
  fly: "flydotio",
  gcloud: "googlecloud",
  helm: "helm",
  k3s: "k3s",
  kubernetes: "kubernetes",
  netlify: "netlify",
  nomad: "nomad",
  opentofu: "opentofu",
  packer: "packer",
  pulumi: "pulumi",
  railway: "railway",
  terraform: "terraform",
  vagrant: "vagrant",
  vault: "vault",
  vercel: "vercel",
  // Network, security and media tools.
  bitwarden: "bitwarden",
  caddy: "caddy",
  curl: "curl",
  ffmpeg: "ffmpeg",
  gpg: "gnuprivacyguard",
  htop: "htop",
  httpie: "httpie",
  nginx: "nginx",
  onepassword: "1password",
  tailscale: "tailscale",
  wireguard: "wireguard",
  wireshark: "wireshark",
  // Linux distributions.
  alma: "almalinux",
  alpine: "alpinelinux",
  arch: "archlinux",
  centos: "centos",
  debian: "debian",
  fedora: "fedora",
  gentoo: "gentoo",
  kali: "kalilinux",
  manjaro: "manjaro",
  mint: "linuxmint",
  opensuse: "opensuse",
  redhat: "redhat",
  rocky: "rockylinux",
  slackware: "slackware",
  ubuntu: "ubuntu",
  void: "voidlinux",
} as const satisfies Readonly<Record<string, BrandGlyphName>>;

export type BrandTile = keyof typeof BRAND_TILES;

/**
 * The palette colours a program known to nothing may be given: the plain and bright
 * colours, without black, white and greys.
 */
const PLAIN_COLOURS: readonly number[] = [1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14];

/** How many colours a program known to nothing may be given. */
export const PLAIN_TILE_COUNT = PLAIN_COLOURS.length;

/** The tile of a program known to nothing, by the colour its name picks. */
export type PlainTile = `plain-${number}`;

/** Every tile there is. */
export type TileId = CategoryTile | BrandTile | PlainTile;

/**
 * A picture a pane is known by that is not one of the tiles: one of a plugin's own, by
 * its key, or one of the page's line drawings, by its name.
 */
export type PictureTile = `image:${string}` | `glyph:${string}`;

const IMAGE = "image:";
const GLYPH = "glyph:";

/** A picture as a pane names it. */
export function pictureTile(picture: IconRef): PictureTile {
  return "glyph" in picture ? `${GLYPH}${picture.glyph}` : `${IMAGE}${picture.image}`;
}

/** The picture a pane names, where what it names is a picture and not a tile. */
export function pictureOf(named: string): IconRef | undefined {
  if (named.startsWith(IMAGE)) return { image: named.slice(IMAGE.length) };
  if (named.startsWith(GLYPH)) return { glyph: named.slice(GLYPH.length) };
  return undefined;
}

/** The tiles the configuration may name for a program. */
export const NAMED_TILES: readonly (CategoryTile | BrandTile)[] = [
  ...CATEGORY_TILES,
  ...(Object.keys(BRAND_TILES) as BrandTile[]).sort(),
];

/** What a tile is drawn with. A tile not known here is drawn as a terminal. */
export function tileOf(id: TileId): Tile {
  if (Object.hasOwn(CATEGORIES, id)) return CATEGORIES[id as CategoryTile];
  if (Object.hasOwn(BRAND_TILES, id)) return brand(BRAND_TILES[id as BrandTile]);
  const plain = /^plain-(\d+)$/.exec(id);
  const picked = plain ? PLAIN_COLOURS[Number(plain[1])] : undefined;
  if (picked !== undefined) return line("terminal", picked, INK);
  return CATEGORIES.terminal;
}
