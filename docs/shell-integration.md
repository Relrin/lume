# Shell integration

A terminal sees only the characters a program prints. It cannot tell a prompt from a
command's output, and it does not know which directory the shell is in. Shell integration
is a small script, loaded into the shell, that says these things in the output itself, with
escape sequences that are never shown.

Lume uses what the shell reports to show a pane's directory, the command that is running and
how the last one ended, and to start a new pane in the directory of the one beside it.

## What the shell reports

Every sequence is an operating system command: `ESC ]`, a number, the text, and `ESC \`.
They are the sequences other terminals use, so a script left in a start-up file does no harm
anywhere: a terminal that does not know a sequence ignores it.

| Sequence                      | When                                 | What it says                                                                                                                                  |
| ----------------------------- | ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `OSC 7 ; file://host/path`    | Before each prompt                   | The shell's working directory, as a URL. The path is percent-encoded. On Windows it is written with its drive, as in `file://box/C:/Users/me` |
| `OSC 133 ; A`                 | Before each prompt                   | A prompt begins here                                                                                                                          |
| `OSC 133 ; B`                 | After each prompt                    | The prompt has ended; what follows is typed                                                                                                   |
| `OSC 133 ; C ; cmdline_url=…` | When a command line has been entered | Its output begins here. The parameter is the command line, percent-encoded; it is left out when the shell cannot say what the line was        |
| `OSC 133 ; D ; code`          | When a command line has finished     | Its exit code. Not sent for a line with nothing on it                                                                                         |
| `OSC 2 ; text`                | With `C` and before each prompt      | The title: the command line while it runs, the directory at the prompt                                                                        |

What a shell reports is a claim, not a fact. Any program can print these sequences, and so
can a file shown with `cat` or a machine at the other end of an SSH connection. Lume shows
what was reported and uses a reported directory for one thing only: where a new pane on
this machine starts, and only if the report names this machine and the directory is there.

## Shells that are set up for you

A profile's shell is set up when the profile has `shell-integration = "auto"`, which is the
default. No start-up file of yours is changed. Each shell is reached the way that shell
offers:

| Shell              | How the script gets in                                                                                                                                                                                                                                                                                                         |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| bash               | bash is started with one line in `PROMPT_COMMAND`, which it runs before its first prompt, after your own start-up files. The line loads the script and removes itself. Whatever your files put in `PROMPT_COMMAND` stays and runs as before                                                                                    |
| zsh                | zsh is started with `ZDOTDIR` naming a folder of Lume's. The `.zshenv` there sets `ZDOTDIR` back to yours, reads your `.zshenv`, and loads the script. Every later start-up file is read from your own folder                                                                                                                  |
| fish               | fish is started with a folder of Lume's at the front of `XDG_DATA_DIRS`. The file fish finds there sets the list back to what it was and loads the script                                                                                                                                                                      |
| PowerShell         | PowerShell has no such way in, so it is given arguments: `-NoExit -Command` and a line that loads the script. This is done only when the profile's own arguments name nothing to run (no `-Command`, `-File` or `-EncodedCommand`). The script is read as text, so it does not depend on the execution policy for script files |
| A WSL distribution | The shell inside is not known from outside, so the distribution is given what bash and zsh need, and the variables are listed in `WSLENV` so that they cross over. fish inside WSL is not set up; see below                                                                                                                    |

Other programs are started exactly as the profile says.

The scripts add themselves around what is there. Your prompt is still drawn by your prompt,
your aliases are untouched, and functions you run before a prompt or before a command
still run and still see the last exit code.

When a hook cannot work, nothing is reported and nothing else changes. That is the case
when a start-up file of yours sets `PROMPT_COMMAND` to something new instead of adding to
it, or when zsh is told its `ZDOTDIR` by `/etc/zsh/zshenv`.

## Turning it off

```toml
[[profile]]
id = "auto.pwsh"
shell-integration = "off"
```

The shell is then started with nothing added: no variable, no argument.

## Setting it up yourself

`lume shell-integration <shell>` prints the script for `bash`, `zsh`, `fish` or
`powershell`. Put it in a file and read that file from the shell's own start-up file. This
is the way for a shell on another machine, for fish inside WSL, and for any shell Lume
does not start itself.

```sh
# bash, on the machine where the shell runs
lume shell-integration bash > ~/.config/lume.bash
echo '. ~/.config/lume.bash' >> ~/.bashrc
```

```sh
# zsh
lume shell-integration zsh > ~/.config/lume.zsh
echo '. ~/.config/lume.zsh' >> ~/.zshrc
```

```fish
# fish
lume shell-integration fish > ~/.config/fish/conf.d/lume.fish
```

```powershell
# PowerShell
lume shell-integration powershell > $HOME\lume.ps1
Add-Content $PROFILE '. $HOME\lume.ps1'
```

A script read this way looks for nothing of Lume's. It only makes sure it is loaded once,
so it may be read by a start-up file and by the automatic setup at the same time.

## What to expect

- **Another integration.** If the prompt is already marked by another terminal's script,
  Lume's script does nothing.
- **bash before 4.4** (the one macOS ships) reports the directory and marks the prompts.
  Commands are not marked: that needs `PS0`, which came with 4.4.
- **bash and the command line.** bash does not hand the line it read to a script, so the
  line is taken from the history. A line the history does not keep (one that begins with a
  blank under `HISTCONTROL=ignorespace`, or one matched by `HISTIGNORE`) is reported as the
  line before it. With the history switched off no line is reported.
- **fish** from version 4 marks prompts and commands itself, and the script then only
  reports the directory. The title is left to fish, which sets it to the running command.
- **PowerShell and exit codes.** A program's exit code is reported as it is. A failure of
  PowerShell's own (a cmdlet that failed, an error in an expression) is reported as 1.
- **PowerShell prompts that draw themselves** with `Write-Host` and return nothing are
  marked too. PowerShell shows `PS>` after such a prompt, with or without the script.
- **Nested shells.** A shell started inside another is not set up, and what it prints is
  the outer shell's running command. If it has the script in its own start-up file, its
  prompts are reported as prompts.
- **Windows.** The console layer between a program and the terminal passes these sequences
  on, and sets titles in its own words: a title arrives a moment after the output around it.
