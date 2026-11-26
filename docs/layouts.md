# Themes, layouts and live reload

Lume reads its configuration folder again whenever a file in it is saved. A script or an editor can
change how Lume looks and what it opens while it runs, and nothing needs restarting.

| Where                    | What                                               | Read again when saved |
| ------------------------ | -------------------------------------------------- | --------------------- |
| `config.toml`            | the settings                                       | yes                   |
| `workspaces/<name>.toml` | a workspace's folder and the settings it overrides | yes                   |
| `themes/<name>.toml`     | your own themes                                    | yes                   |
| `layouts/<name>.toml`    | tabs of panes to open by name                      | yes                   |

The configuration folder is `%APPDATA%\lume` on Windows and `~/.config/lume` on Linux and macOS, or
the folder `LUME_CONFIG_DIR` names. The "Open config.toml" button at the top of the settings window
opens the file.

A file that is not TOML is never half applied. The configuration from before stays in use, and the
status bar says `Config not applied` until the file is fixed. Click it to see each problem with its
file and line, and to open the file. A key Lume does not know, or a value it cannot use, is passed
over and the rest of the file applies. The status bar then shows a quieter `Config · 1 problem`.

## Themes

Lume has four built-in themes:

| Id              | Name          | Scheme |
| --------------- | ------------- | ------ |
| `lume-dark`     | Lume Dark     | dark   |
| `lume-midnight` | Lume Midnight | dark   |
| `lume-light`    | Lume Light    | light  |
| `lume-daylight` | Lume Daylight | light  |

Lume Midnight and Lume Daylight are adapted from the "Cursor Dark Midnight" and "Cursor Light"
themes by BioHazard786 ([cursor-theme-vscode](https://github.com/BioHazard786/cursor-theme-vscode),
MIT licence).

Choose one in Settings → Appearance, or in `config.toml`:

```toml
[appearance]
theme-dark = "lume-midnight"   # used while the system is dark
theme-light = "lume-daylight"  # used while the system is light
# theme = "lume-midnight"      # this one whatever the system is
```

A theme of your own is a file in `themes/` that sets only the tokens it changes:

```toml
# themes/midnight-warm.toml
name = "Midnight, warm"
inherits = "lume-midnight"

[color]
accent = "#d08770"
```

Save the file and every window shows the change. The theme is added to the picker as soon as the
file is there. `lume-dark.toml` in [the source](../crates/lume-config/src/themes/lume-dark.toml)
lists every token.

## Layout files

A layout file describes tabs of panes, what each pane runs and where. Open one from the new-tab menu
(the arrow beside `+`, under **Layouts**), or from a script:

```powershell
lume ctl layout files          # the layouts there are, and what is wrong with any
lume ctl layout open dev       # opens layouts/dev.toml in the window last worked in
```

The quickest way to make one is from a tab you already have. Open the tab's menu, choose
**Save as layout**, then **Open file** in the notice that follows. The file looks like this:

```toml
# layouts/dev.toml
name = "Lume dev"
description = "Editor, checks and logs"
color = "blue"            # the colour of the group, for a layout of several tabs
cwd = "~/src/lume"        # where every pane starts, unless it says otherwise

[[tab]]
title = "code"
split = "row"             # side by side; "column" puts them one above the other
sizes = [60, 40]          # shares, in any unit: these are parts of their sum
focus = "editor"          # the pane that has the keyboard

  [[tab.pane]]
  key = "editor"
  command = "nvim ."

  [[tab.pane]]
  split = "column"
    [[tab.pane.pane]]
    key = "check"
    command = ["cargo", "watch", "-x", "check"]
    env = { RUST_LOG = "debug" }
    [[tab.pane.pane]]
    key = "shell"          # no command: the default shell, or `profile = "<id>"`

[[tab]]
title = "logs"
command = "tail -f app.log"
```

- Editors that read JSON Schemas for TOML, such as Taplo or Even Better TOML, can complete and
  check a layout file with [`schemas/layout.schema.json`](../schemas/layout.schema.json). Put
  `#:schema <path to it>` on the file's first line.
- A tab is one pane, or a `split` with `[[pane]]`s inside it. Splits nest.
- `command` is a program and its arguments, as a list or as one line split at spaces outside
  quotes. It runs directly, not through a shell. To keep a shell open after the program ends, run
  the shell: `["pwsh", "-NoExit", "-Command", "npm run dev"]`.
- `profile` starts the pane the way a profile does. `lume ctl profiles` lists their ids.
- `cwd` and `env` apply to the panes under them, and the innermost setting wins. A leading `~` is
  your home folder.
- `plugin` and `view` show a plugin's view instead of a terminal.
- A layout with one tab opens as a tab. A layout with several opens as a group named after the
  layout.

### Changing a layout that is open

A tab opened from a layout file remembers the file. Save the file and Lume notices within a moment:

- **Ask**, the default: the window offers to apply the change, and says what would happen to the
  panes the file no longer has.
- **Always**: the change is applied straight away. This suits scripts.
- **Never**: open tabs are left alone.

Change this in Settings → Appearance → Panes → "When a layout file changes", or with
`[layout] files-reapply = "ask" | "always" | "never"`.

Panes are matched by their `key`. A pane whose key is still in the file keeps its running program
and its scrollback, wherever it moves. A pane without a `key` is matched by its place in the file,
so give panes keys if you rearrange them. When a pane leaves the file:

- if it is idle, it closes;
- if a program is still running in it, it is put away (minimised), never ended. It stays in the
  window's tray until you bring it back or close it.

The tab's menu shows **From layout "…"**, which has these entries:

- **Open layout file**
- **Apply file again**
- **Stop following file**: the tab stays as it is and stops tracking the file.

A tab opened from a layout of one tab can be written back with **Save to layout "…"**.

### A script that drives a layout

```powershell
# Writes a layout with one pane per service, and opens it.
$services = "api", "web", "worker"
$dir = Join-Path $env:APPDATA "lume\layouts"
New-Item -ItemType Directory -Force $dir | Out-Null

$lines = @('name = "Services"', '', '[[tab]]', 'title = "services"', 'split = "column"')
foreach ($service in $services) {
    $lines += '', '  [[tab.pane]]', "  key = `"$service`"", "  command = [`"docker`", `"compose`", `"logs`", `"-f`", `"$service`"]"
}
Set-Content -Path (Join-Path $dir "services.toml") -Value $lines -Encoding utf8

lume ctl layout open services
```

Run it again with a different list of services. The open tab gains and loses panes to match, and
the panes of services that are still listed keep running.
