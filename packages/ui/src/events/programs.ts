/**
 * The picture a pane shows for the program it runs.
 *
 * Programs are known by their names, without their folder or `.exe`, compared without
 * regard to case. The user's own choices, from `appearance.program-icons`, come before
 * the ones known here; a program neither knows is shown as a terminal.
 */

/** The pictures there are for a program. The configuration names them the same way. */
export const PROGRAM_ICONS = [
  "terminal",
  "editor",
  "git",
  "monitor",
  "agent",
  "remote",
  "container",
  "files",
] as const;

/** A picture for a program. */
export type ProgramIcon = (typeof PROGRAM_ICONS)[number];

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

/** The programs known here, by picture. */
const KNOWN: Readonly<Record<ProgramIcon, readonly string[]>> = {
  terminal: [...SHELLS, "tmux", "screen", "zellij"],
  editor: [
    "vim",
    "nvim",
    "vi",
    "view",
    "nano",
    "pico",
    "micro",
    "emacs",
    "hx",
    "helix",
    "kak",
    "less",
    "more",
    "most",
    "bat",
    "man",
    "ed",
  ],
  git: ["git", "lazygit", "tig", "gitui", "gh", "glab", "hg", "jj", "svn"],
  monitor: [
    "top",
    "htop",
    "btop",
    "btm",
    "atop",
    "glances",
    "nvtop",
    "iotop",
    "watch",
    "procs",
    "gotop",
  ],
  agent: ["claude", "codex", "aider", "gemini", "goose", "opencode", "amp"],
  remote: ["ssh", "mosh", "telnet", "et", "sftp", "scp", "rsync"],
  container: [
    "docker",
    "podman",
    "kubectl",
    "k9s",
    "lazydocker",
    "nerdctl",
    "vagrant",
    "multipass",
  ],
  files: ["ranger", "lf", "nnn", "yazi", "mc", "broot", "vifm", "far"],
};

/** The picture of every known program, by its name. */
const BY_NAME: ReadonlyMap<string, ProgramIcon> = new Map(
  PROGRAM_ICONS.flatMap((icon) => KNOWN[icon].map((name) => [name, icon] as const)),
);

/** A program's name as it is compared: no folder, no `.exe`, lower case. */
export function programKey(program: string): string {
  const base = program.split(/[\\/]/).at(-1) ?? program;
  return base.replace(/\.exe$/i, "").toLowerCase();
}

/** Whether a program is a shell, by its name. */
export function isShell(program: string): boolean {
  return SHELLS.includes(programKey(program));
}

/** Whether a value is the name of a picture. */
export function isProgramIcon(value: unknown): value is ProgramIcon {
  return typeof value === "string" && (PROGRAM_ICONS as readonly string[]).includes(value);
}

/** The user's choices as the configuration holds them: only the ones that name a picture. */
export function iconChoices(written: unknown): ReadonlyMap<string, ProgramIcon> {
  const choices = new Map<string, ProgramIcon>();
  if (typeof written !== "object" || written === null) return choices;
  for (const [name, icon] of Object.entries(written)) {
    if (isProgramIcon(icon)) choices.set(programKey(name), icon);
  }
  return choices;
}

/** The picture for a program, or nothing when neither the user nor this list knows it. */
export function programIcon(
  program: string,
  choices: ReadonlyMap<string, ProgramIcon>,
): ProgramIcon | undefined {
  const key = programKey(program);
  return choices.get(key) ?? BY_NAME.get(key);
}
