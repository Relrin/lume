/**
 * The tile a pane shows for the program it runs.
 *
 * Programs are known by their names, without their folder or `.exe`, compared without
 * regard to case. The user's own choices, from `appearance.program-icons`, come before
 * the ones known here. A well-known program has a tile of its own, others a tile for
 * their kind; a program known for neither is given a plain tile by its name, the same
 * one each time.
 */

import {
  NAMED_TILES,
  PLAIN_TILE_COUNT,
  type BrandTile,
  type CategoryTile,
  type PlainTile,
} from "../icons/tiles.ts";

/** The tiles the configuration may name for a program. */
export const PROGRAM_ICONS = NAMED_TILES;

/** A tile the configuration may name for a program. */
export type ProgramIcon = CategoryTile | BrandTile;

/** The tile a pane shows: one the configuration may name, or a plain one. */
export type PaneIcon = ProgramIcon | PlainTile;

/** The shells known here: what a terminal runs at its prompt. */
const SHELLS: readonly string[] = [
  "bash",
  "zsh",
  "fish",
  "sh",
  "dash",
  "ksh",
  "tcsh",
  "csh",
  "nu",
  "elvish",
  "xonsh",
  "pwsh",
  "powershell",
  "cmd",
];

/** The programs known here by their kind. */
const KINDS: Readonly<Record<CategoryTile, readonly string[]>> = {
  terminal: [...SHELLS, "screen", "zellij", "byobu"],
  editor: ["nano", "pico", "micro", "kak", "less", "more", "most", "bat", "man", "ed"],
  git: ["git", "lazygit", "tig", "gitui", "jj", "git-lfs", "delta"],
  monitor: ["top", "btop", "btm", "atop", "glances", "nvtop", "iotop", "watch", "procs", "gotop"],
  agent: ["codex", "aider", "goose", "amp", "crush", "kiro-cli", "q", "auggie", "droid"],
  remote: ["ssh", "mosh", "telnet", "et", "sftp", "scp", "rsync"],
  container: ["podman", "podman-compose", "nerdctl", "multipass", "lima", "limactl", "colima"],
  files: ["ranger", "lf", "nnn", "yazi", "mc", "broot", "vifm", "far"],
};

/**
 * The programs known here by their own tile. Linux distributions are among them, by
 * the names WSL gives them. A shell keeps the prompt's tile unless the configuration
 * names its mark, so the shells' marks have no names here.
 */
const OWN: Readonly<Record<BrandTile, readonly string[]>> = {
  // Shells and multiplexers.
  bash: [],
  fish: [],
  nushell: [],
  tmux: ["tmux"],
  zsh: [],
  // Editors.
  emacs: ["emacs", "emacsclient"],
  helix: ["hx", "helix"],
  lapce: ["lapce"],
  neovim: ["nvim", "neovim"],
  obsidian: ["obsidian", "obsidian-cli"],
  sublime: ["subl", "sublime_text"],
  vim: ["vim", "vi", "view", "vimdiff", "gvim"],
  vscodium: ["codium", "vscodium"],
  zed: ["zed", "zeditor"],
  // Version control and forges.
  forgejo: ["forgejo", "fj"],
  fossil: ["fossil"],
  gitea: ["tea", "gitea"],
  github: ["gh"],
  gitlab: ["glab"],
  mercurial: ["hg", "chg"],
  subversion: ["svn", "svnadmin"],
  // Coding agents and AI providers.
  claude: ["claude", "claude-code"],
  cline: ["cline"],
  coderabbit: ["coderabbit"],
  copilot: ["copilot", "github-copilot-cli"],
  cursor: ["cursor-agent", "cursor"],
  gemini: ["gemini"],
  huggingface: ["hf", "huggingface-cli"],
  lmstudio: ["lms"],
  mistral: ["vibe", "mistral"],
  ollama: ["ollama"],
  opencode: ["opencode"],
  qwen: ["qwen"],
  // Databases.
  bigquery: ["bq"],
  cassandra: ["cqlsh", "cassandra"],
  clickhouse: ["clickhouse", "clickhouse-client", "clickhouse-local"],
  cockroachdb: ["cockroach"],
  duckdb: ["duckdb"],
  elasticsearch: ["elasticsearch"],
  firebase: ["firebase"],
  influxdb: ["influx", "influxd"],
  mariadb: ["mariadb", "mariadbd"],
  mongodb: ["mongosh", "mongo", "mongod"],
  mysql: ["mysql", "mysqld", "mysqlsh", "mycli"],
  neo4j: ["neo4j", "cypher-shell"],
  planetscale: ["pscale"],
  postgres: ["psql", "postgres", "pgcli", "pg_dump", "pg_restore"],
  prisma: ["prisma"],
  redis: ["redis-cli", "redis-server"],
  snowflake: ["snowsql", "snow"],
  sqlite: ["sqlite3", "sqlite", "litecli"],
  supabase: ["supabase"],
  surrealdb: ["surreal"],
  turso: ["turso"],
  // Languages, runtimes and build tools.
  bazel: ["bazel", "bazelisk"],
  bun: ["bun", "bunx"],
  clojure: ["clojure", "clj", "lein"],
  cmake: ["cmake", "ctest"],
  crystal: ["crystal", "shards"],
  dart: ["dart"],
  deno: ["deno"],
  dotnet: ["dotnet"],
  elixir: ["elixir", "iex", "mix"],
  erlang: ["erl", "rebar3"],
  flutter: ["flutter"],
  gleam: ["gleam"],
  go: ["go", "gopls"],
  gradle: ["gradle", "gradlew"],
  haskell: ["ghc", "ghci", "cabal", "stack"],
  java: ["java", "javac", "jshell", "mvn", "mvnw"],
  julia: ["julia"],
  jupyter: ["jupyter", "jupyter-lab", "jupyter-notebook"],
  kotlin: ["kotlin", "kotlinc"],
  lua: ["lua", "luajit"],
  nim: ["nim", "nimble"],
  node: ["node", "npx", "tsx", "ts-node"],
  ocaml: ["ocaml", "utop", "dune", "opam"],
  perl: ["perl"],
  php: ["php", "composer"],
  python: ["python", "py", "ipython", "pip", "pytest"],
  r: ["r", "rscript", "radian"],
  racket: ["racket", "raco"],
  rails: ["rails"],
  ruby: ["ruby", "irb", "gem", "bundle", "rake"],
  rust: ["cargo", "rustc", "rustup", "rust-analyzer"],
  scala: ["scala", "scala-cli", "sbt"],
  swift: ["swift", "swiftc"],
  typescript: ["tsc"],
  vite: ["vite"],
  zig: ["zig"],
  // Package managers.
  chocolatey: ["choco"],
  conda: ["conda", "mamba", "micromamba"],
  homebrew: ["brew"],
  nix: ["nix", "nix-shell", "nix-env", "nixos-rebuild", "nixos"],
  npm: ["npm"],
  pnpm: ["pnpm"],
  poetry: ["poetry"],
  uv: ["uv", "uvx"],
  yarn: ["yarn"],
  // Containers, clouds and infrastructure.
  ansible: ["ansible", "ansible-playbook", "ansible-galaxy"],
  argo: ["argo", "argocd"],
  cloudflare: ["wrangler", "cloudflared"],
  consul: ["consul"],
  digitalocean: ["doctl"],
  docker: ["docker", "docker-compose", "lazydocker", "dockerd"],
  fly: ["fly", "flyctl"],
  gcloud: ["gcloud", "gsutil"],
  helm: ["helm"],
  k3s: ["k3s", "k3d"],
  kubernetes: ["kubectl", "k9s", "kubectx", "kubens", "kustomize", "minikube", "kind"],
  netlify: ["netlify"],
  nomad: ["nomad"],
  opentofu: ["tofu"],
  packer: ["packer"],
  pulumi: ["pulumi"],
  railway: ["railway"],
  terraform: ["terraform"],
  vagrant: ["vagrant"],
  vault: ["vault"],
  vercel: ["vercel"],
  // Network, security and media tools.
  bitwarden: ["bw"],
  caddy: ["caddy"],
  curl: ["curl"],
  ffmpeg: ["ffmpeg", "ffprobe", "ffplay"],
  gpg: ["gpg", "gpg2"],
  htop: ["htop"],
  httpie: ["http", "https", "httpie"],
  nginx: ["nginx"],
  onepassword: ["op"],
  tailscale: ["tailscale", "tailscaled"],
  wireguard: ["wg", "wg-quick"],
  wireshark: ["tshark", "wireshark"],
  // Linux distributions.
  alma: ["alma", "almalinux"],
  alpine: ["alpine"],
  arch: ["arch", "archlinux"],
  centos: ["centos"],
  debian: ["debian"],
  fedora: ["fedora", "fedoralinux"],
  gentoo: ["gentoo"],
  kali: ["kali", "kali-linux", "kalilinux"],
  manjaro: ["manjaro"],
  mint: ["mint", "linuxmint"],
  opensuse: ["opensuse", "opensuse-leap", "opensuse-tumbleweed"],
  redhat: ["rhel", "redhat", "redhatenterpriselinux"],
  rocky: ["rocky", "rockylinux"],
  slackware: ["slackware"],
  ubuntu: ["ubuntu"],
  void: ["void", "voidlinux"],
};

/** The tile of every known program, by its name: its own before its kind's. */
const BY_NAME: ReadonlyMap<string, ProgramIcon> = new Map([
  ...(Object.keys(KINDS) as CategoryTile[]).flatMap((kind) =>
    KINDS[kind].map((name) => [name, kind] as const),
  ),
  ...(Object.keys(OWN) as BrandTile[]).flatMap((tile) =>
    OWN[tile].map((name) => [name, tile] as const),
  ),
]);

/** A program's name as it is compared: no folder, no `.exe`, lower case. */
export function programKey(program: string): string {
  const base = program.split(/[\\/]/).at(-1) ?? program;
  return base.replace(/\.exe$/i, "").toLowerCase();
}

/** Whether a program is a shell, by its name. */
export function isShell(program: string): boolean {
  return SHELLS.includes(programKey(program));
}

/**
 * The programs a person works in, as opposed to those they wait for: coding agents,
 * editors and pagers, monitors, file managers, multiplexers and shells on other
 * machines. One of them at the prompt is the shell's command for as long as it is
 * open, which says nothing of whether anything is under way.
 */
const WORKED_IN: ReadonlySet<string> = new Set([
  ...KINDS.agent,
  ...OWN.claude,
  ...OWN.cline,
  ...OWN.copilot,
  ...OWN.cursor,
  ...OWN.gemini,
  ...OWN.opencode,
  ...OWN.qwen,
  "pi",
  "agy",
  ...KINDS.editor,
  ...OWN.emacs,
  ...OWN.helix,
  ...OWN.neovim,
  ...OWN.vim,
  ...KINDS.monitor,
  ...KINDS.files,
  "tmux",
  "screen",
  "zellij",
  "byobu",
  "lazygit",
  "tig",
  "gitui",
  "ssh",
  "mosh",
  "telnet",
]);

/** Whether a program is one a person works in, by its name. */
export function isWorkedIn(program: string): boolean {
  return WORKED_IN.has(programKey(program));
}

/** Whether a value is the name of a tile the configuration may name. */
export function isProgramIcon(value: unknown): value is ProgramIcon {
  return typeof value === "string" && (PROGRAM_ICONS as readonly string[]).includes(value);
}

/** The user's choices as the configuration holds them: only the ones that name a tile. */
export function iconChoices(written: unknown): ReadonlyMap<string, ProgramIcon> {
  const choices = new Map<string, ProgramIcon>();
  if (typeof written !== "object" || written === null) return choices;
  for (const [name, icon] of Object.entries(written)) {
    if (isProgramIcon(icon)) choices.set(programKey(name), icon);
  }
  return choices;
}

/**
 * The names a program may be known by, most exact first: as it is, without a version
 * at its end (`python3.12`, `Ubuntu-24.04`), and the first word of a name joined with
 * dashes (`redis-cli`, `openSUSE-Leap-15.6`). A name cut down to one letter is not
 * tried: `r2` is not R.
 */
function namesOf(key: string): readonly string[] {
  const unversioned = key.replace(/[-_]?v?\d+(\.\d+)*$/, "");
  const first = unversioned.split("-")[0] ?? unversioned;
  return [key, unversioned, first].filter((name, at) => at === 0 || name.length > 1);
}

/** The tile known here for a name, without the user's choices. */
function knownTile(key: string): ProgramIcon | undefined {
  for (const name of namesOf(key)) {
    const tile = BY_NAME.get(name);
    if (tile) return tile;
  }
  return undefined;
}

/**
 * The tile for a program, or nothing when neither the user nor this list knows it.
 * `app` is what the program said it is, where it said: it is known before the
 * program's name, but after the user's choices.
 */
export function programIcon(
  program: string,
  choices: ReadonlyMap<string, ProgramIcon>,
  app?: string,
): ProgramIcon | undefined {
  const key = programKey(program);
  const chosen = choices.get(key);
  if (chosen) return chosen;
  const said = app === undefined ? undefined : knownTile(programKey(app));
  return said ?? knownTile(key);
}

/** The tile of a Linux distribution, by the name WSL gives it, if it is known. */
export function distributionIcon(distribution: string): BrandTile | undefined {
  const tile = knownTile(programKey(distribution));
  return tile !== undefined && Object.hasOwn(OWN, tile) ? (tile as BrandTile) : undefined;
}

/** The plain tile a program known to nothing is given, picked by its name. */
export function plainIcon(program: string): PlainTile {
  // FNV-1a over the name's UTF-16 code units: small, and the same everywhere.
  const key = programKey(program);
  let hash = 0x811c9dc5;
  for (let at = 0; at < key.length; at += 1) {
    hash ^= key.charCodeAt(at);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return `plain-${hash % PLAIN_TILE_COUNT}`;
}
