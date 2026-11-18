//! Shell integration: having a shell report its directory and its commands.
//!
//! A shell says these things with escape sequences in its output, and it says them
//! only if a script of ours is loaded into it. The scripts are part of this program and
//! are written to a folder when the first session wants them. A shell is then made to
//! load its script through its environment alone, in the way that shell offers, and
//! never by touching a start-up file of the user's:
//!
//! - **bash** runs `PROMPT_COMMAND` before each prompt. It is started with one line
//!   there, which loads the script before the first prompt and takes itself out.
//! - **zsh** reads its start-up files from `ZDOTDIR`. It is started with that naming a
//!   folder of ours, whose `.zshenv` puts the user's folder back, reads the user's
//!   `.zshenv`, and loads the script.
//! - **fish** reads `vendor_conf.d` folders under `XDG_DATA_DIRS`. It is started with
//!   a folder of ours at the front of that list, whose file puts the list back and
//!   loads the script.
//! - **PowerShell** has no such way in, so it alone is given arguments: `-NoExit
//!   -Command` and a line that loads the script, and only when its own arguments name
//!   nothing to run.
//!
//! Every one of these does nothing unless [`ENABLED`] is set, which only a launch
//! from here sets. A shell inside a Linux distribution is not known, so a distribution
//! is given what bash and zsh need, and the folder crosses over as a path of its own.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

use lume_pty::CommandSpec;

/// Set for a shell that is to load its script. The scripts' own loaders look for it.
pub(crate) const ENABLED: &str = "LUME_SHELL_INTEGRATION";

/// What [`ENABLED`] is set to. Only its being set matters.
const ENABLED_VALUE: &str = "1";

/// Names the folder the scripts are in, for the loaders.
pub(crate) const SCRIPTS: &str = "LUME_SHELL_INTEGRATION_DIR";

const PROMPT_COMMAND: &str = "PROMPT_COMMAND";

/// The line bash is started with in `PROMPT_COMMAND`. The file it reads takes this
/// very line out again, so the two must agree to the letter.
const BASH_BOOTSTRAP: &str = r#"[ -r "${LUME_SHELL_INTEGRATION_DIR-}/bash/bootstrap.bash" ] && . "$LUME_SHELL_INTEGRATION_DIR/bash/bootstrap.bash""#;

const ZDOTDIR: &str = "ZDOTDIR";

/// The folder among the scripts that zsh is pointed at.
const ZSH_DIR: &str = "zsh";

/// Carries the user's own `ZDOTDIR` to the `.zshenv` that puts it back.
const ZSH_USER_DIR: &str = "LUME_ZSH_ZDOTDIR";

const XDG_DATA_DIRS: &str = "XDG_DATA_DIRS";

/// The folder among the scripts that is put on fish's list of data folders.
const FISH_DATA_DIR: &str = "fish-data";

/// Carries the user's own `XDG_DATA_DIRS` to the file that puts it back.
const FISH_USER_DATA_DIRS: &str = "LUME_FISH_XDG_DATA_DIRS";

/// What `XDG_DATA_DIRS` means when it is not set. A list that held only our folder
/// would take these away from fish.
const XDG_DATA_DIRS_DEFAULT: &str = "/usr/local/share:/usr/share";

/// What PowerShell is asked to run before its first prompt. It reads the script as
/// text, so it does not depend on whether script files may be run, and it says nothing
/// when that fails. It holds no double quote, which a command line would have to
/// escape.
const POWERSHELL_PREAMBLE: &str = "try { if ($env:LUME_SHELL_INTEGRATION) { \
     . ([scriptblock]::Create((Get-Content -Raw -LiteralPath \
     (Join-Path $env:LUME_SHELL_INTEGRATION_DIR 'lume.ps1')))) } } catch {}";

/// The marks in a `WSLENV` entry that have a path rewritten as it crosses.
const WSLENV_PATH: &str = "/p";

/// Every file that is written out, by its place under the scripts' folder.
const FILES: [(&str, &str); 7] = [
    (
        "lume.bash",
        include_str!("../../../resources/shell-integration/lume.bash"),
    ),
    (
        "lume.zsh",
        include_str!("../../../resources/shell-integration/lume.zsh"),
    ),
    (
        "lume.fish",
        include_str!("../../../resources/shell-integration/lume.fish"),
    ),
    (
        "lume.ps1",
        include_str!("../../../resources/shell-integration/lume.ps1"),
    ),
    (
        "bash/bootstrap.bash",
        include_str!("../../../resources/shell-integration/bash/bootstrap.bash"),
    ),
    (
        "zsh/.zshenv",
        include_str!("../../../resources/shell-integration/zsh/.zshenv"),
    ),
    (
        "fish-data/fish/vendor_conf.d/lume-shell-integration.fish",
        include_str!(
            "../../../resources/shell-integration/fish-data/fish/vendor_conf.d/lume-shell-integration.fish"
        ),
    ),
];

/// The two numbers of the 64-bit FNV-1a hash.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A file as it is written: with the line endings shells on every system read. A
/// checkout may have given the embedded text another kind.
fn written(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// A name for what the files hold, which changes when any of them does.
fn content_name(files: &[(&str, String)]) -> String {
    let mut hash = FNV_OFFSET;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes.iter().chain(&[0]) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    };
    for (path, text) in files {
        feed(path.as_bytes());
        feed(text.as_bytes());
    }
    format!("{hash:016x}")
}

/// Writes the scripts under `root`, in a folder named for what they hold, and says
/// which folder that is.
///
/// A file that is already there with the same text is left alone, so starting again
/// writes nothing, and two programs doing this at once write the same thing. A folder
/// left by another version of the scripts is left too: a shell started from it may
/// still be running.
pub(crate) fn install(root: &Path) -> io::Result<PathBuf> {
    let files: Vec<(&str, String)> = FILES
        .iter()
        .map(|(path, text)| (*path, written(text)))
        .collect();
    let folder = root.join(content_name(&files));
    for (path, text) in &files {
        let file = path
            .split('/')
            .fold(folder.clone(), |at, part| at.join(part));
        if std::fs::read(&file).is_ok_and(|there| there == text.as_bytes()) {
            continue;
        }
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Written beside its place and moved there, so that a shell starting at this
        // moment reads the whole file or none of it.
        let mut partial = file.clone().into_os_string();
        partial.push(format!(".{}.partial", std::process::id()));
        std::fs::write(&partial, text)?;
        if let Err(error) = std::fs::rename(&partial, &file) {
            let _ = std::fs::remove_file(&partial);
            return Err(error);
        }
    }
    Ok(folder)
}

/// The shells that can be made to load a script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

/// Which shell a program is, told from its file's name.
fn shell_of(program: &str) -> Option<Kind> {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let lower = file.to_ascii_lowercase();
    match lower.strip_suffix(".exe").unwrap_or(&lower) {
        "bash" => Some(Kind::Bash),
        "zsh" => Some(Kind::Zsh),
        "fish" => Some(Kind::Fish),
        "pwsh" | "pwsh-preview" | "powershell" => Some(Kind::PowerShell),
        _ => None,
    }
}

/// What a session has a variable set to: by itself, or else as this program has it.
fn current(
    command: &CommandSpec,
    name: &str,
    inherited: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let own = command.env.iter().rev().find(|(set, _)| set == name);
    match own {
        Some((_, value)) => Some(value.to_string_lossy().into_owned()),
        None if command.env_remove.iter().any(|gone| gone == name) => None,
        None => inherited(name),
    }
}

fn set(env: &mut Vec<(OsString, OsString)>, name: &str, value: impl Into<OsString>) {
    env.retain(|(set, _)| set != name);
    env.push((name.into(), value.into()));
}

/// The line bash is started with, before whatever the session already runs there.
fn bash_prompt_command(already: Option<&str>) -> String {
    match already.filter(|already| !already.trim().is_empty()) {
        // A line of its own: a new line may follow anything a command may end with.
        Some(already) => format!("{BASH_BOOTSTRAP}\n{already}"),
        None => BASH_BOOTSTRAP.to_owned(),
    }
}

/// How PowerShell's own parser matches a switch: any beginning of the name that is
/// at least as long as the shortest one it accepts.
fn switch_is(word: &str, name: &str, shortest: usize) -> bool {
    word.len() >= shortest && name.starts_with(word)
}

/// Whether PowerShell, given these arguments, will sit at a prompt with nothing of
/// its own to run, so that a command to run first may be added behind them.
///
/// Only arguments known to leave it there count. Anything else (something to run, a
/// request for the version, a word this does not know) leaves the arguments alone.
fn powershell_takes_preamble(args: &[OsString]) -> bool {
    // Switches that stand alone, and switches that take the next argument, each with
    // the shortest spelling PowerShell accepts.
    const ALONE: [(&str, usize); 6] = [
        ("nologo", 3),
        ("noexit", 3),
        ("noprofile", 3),
        ("sta", 3),
        ("mta", 3),
        ("login", 1),
    ];
    const WITH_VALUE: [(&str, usize); 8] = [
        ("executionpolicy", 2),
        ("workingdirectory", 2),
        ("windowstyle", 1),
        ("inputformat", 2),
        ("outputformat", 1),
        ("configurationname", 6),
        ("custompipename", 3),
        ("settingsfile", 8),
    ];
    const WITH_VALUE_ALIASES: [&str; 5] = ["ep", "wd", "if", "of", "wo"];

    let mut words = args.iter();
    while let Some(arg) = words.next() {
        let Some(arg) = arg.to_str() else {
            return false;
        };
        let lower = arg.to_ascii_lowercase();
        let Some(word) = lower
            .strip_prefix("--")
            .or_else(|| lower.strip_prefix('-'))
            .or_else(|| lower.strip_prefix('/'))
            .filter(|word| !word.is_empty())
        else {
            // A word that is no switch is a file or a command to run.
            return false;
        };
        // What names something to run is asked first, as PowerShell asks it: `-c` is
        // a command although other names begin with that letter too.
        let runs = [("command", 1), ("file", 1), ("encodedcommand", 1)]
            .iter()
            .any(|(name, shortest)| switch_is(word, name, *shortest))
            || matches!(word, "ec" | "cwa" | "commandwithargs");
        if runs {
            return false;
        }
        if ALONE
            .iter()
            .any(|(name, shortest)| switch_is(word, name, *shortest))
        {
            continue;
        }
        let valued = WITH_VALUE_ALIASES.contains(&word)
            || WITH_VALUE
                .iter()
                .any(|(name, shortest)| switch_is(word, name, *shortest));
        if valued && words.next().is_some() {
            continue;
        }
        return false;
    }
    true
}

/// Makes the shell a session starts load its script, if `program` is a shell that can
/// be made to. Any other program is left exactly as it was.
///
/// `inherited` reads a variable of this program's own environment, which the session
/// starts with. `windows` says whether the shell runs on Windows.
pub(crate) fn hook_shell(
    command: &mut CommandSpec,
    program: &str,
    scripts: &Path,
    windows: bool,
    inherited: &dyn Fn(&str) -> Option<String>,
) {
    let Some(shell) = shell_of(program) else {
        return;
    };
    match shell {
        Kind::Bash => {
            let already = current(command, PROMPT_COMMAND, inherited);
            set(
                &mut command.env,
                PROMPT_COMMAND,
                bash_prompt_command(already.as_deref()),
            );
        }
        Kind::Zsh => {
            match current(command, ZDOTDIR, inherited) {
                Some(user) => set(&mut command.env, ZSH_USER_DIR, user),
                // One left over from somewhere else would be put in the user's place.
                None => command.env_remove.push(ZSH_USER_DIR.into()),
            }
            set(&mut command.env, ZDOTDIR, scripts.join(ZSH_DIR));
        }
        Kind::Fish => {
            // The list is separated by colons, which a Windows path has in it.
            if windows {
                return;
            }
            let user = current(command, XDG_DATA_DIRS, inherited);
            let mut list = scripts.join(FISH_DATA_DIR).into_os_string();
            list.push(":");
            list.push(
                user.as_deref()
                    .filter(|user| !user.is_empty())
                    .unwrap_or(XDG_DATA_DIRS_DEFAULT),
            );
            match user {
                Some(user) => set(&mut command.env, FISH_USER_DATA_DIRS, user),
                None => command.env_remove.push(FISH_USER_DATA_DIRS.into()),
            }
            set(&mut command.env, XDG_DATA_DIRS, list);
        }
        Kind::PowerShell => {
            if !powershell_takes_preamble(&command.args) {
                return;
            }
            command.args.push("-NoExit".into());
            command.args.push("-Command".into());
            command.args.push(POWERSHELL_PREAMBLE.into());
        }
    }
    set(&mut command.env, ENABLED, ENABLED_VALUE);
    set(&mut command.env, SCRIPTS, scripts);
}

/// The name a `WSLENV` entry lists, without the marks behind its slash.
pub(crate) fn listed_name(entry: &str) -> &str {
    entry.split('/').next().unwrap_or(entry)
}

/// Makes the shell of a Linux distribution load its script, whichever of bash and zsh
/// it turns out to be, and returns the entries `WSLENV` needs for it: the variables
/// set here reach the distribution only if they are listed there.
///
/// `env` is what the session sets. `listed` is the user's own `WSLENV`. A variable the
/// user already has cross over, or the session sets for itself, is theirs: the hook
/// that needs it is left out, since ours would arrive untranslated or in place of
/// theirs.
pub(crate) fn hook_distro(
    env: &mut Vec<(OsString, OsString)>,
    scripts: &Path,
    listed: Option<&str>,
) -> Vec<String> {
    let users = |name: &str| {
        listed.is_some_and(|listed| listed.split(':').any(|entry| listed_name(entry) == name))
    };
    let sets = |env: &[(OsString, OsString)], name: &str| {
        env.iter()
            .any(|(set, _)| set.as_os_str() == OsStr::new(name))
    };

    let mut entries = vec![ENABLED.to_owned(), format!("{SCRIPTS}{WSLENV_PATH}")];
    if !users(PROMPT_COMMAND) {
        let already = env
            .iter()
            .rev()
            .find(|(set, _)| set == PROMPT_COMMAND)
            .map(|(_, value)| value.to_string_lossy().into_owned());
        set(env, PROMPT_COMMAND, bash_prompt_command(already.as_deref()));
        entries.push(PROMPT_COMMAND.to_owned());
    }
    if !users(ZDOTDIR) && !sets(env, ZDOTDIR) {
        set(env, ZDOTDIR, scripts.join(ZSH_DIR));
        entries.push(format!("{ZDOTDIR}{WSLENV_PATH}"));
    }
    set(env, ENABLED, ENABLED_VALUE);
    set(env, SCRIPTS, scripts);
    entries
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn nothing(_: &str) -> Option<String> {
        None
    }

    fn scripts() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\state\shell-integration\0123")
        } else {
            PathBuf::from("/state/shell-integration/0123")
        }
    }

    fn value(command: &CommandSpec, name: &str) -> Option<String> {
        command
            .env
            .iter()
            .find(|(set, _)| set == name)
            .map(|(_, value)| value.to_string_lossy().into_owned())
    }

    fn hooked(program: &str, args: &[&str]) -> CommandSpec {
        let mut command = CommandSpec::new(program).args(args.iter().copied());
        hook_shell(&mut command, program, &scripts(), cfg!(windows), &nothing);
        command
    }

    fn words(command: &CommandSpec) -> Vec<String> {
        command
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_shell_is_known_by_its_files_name() {
        for (program, shell) in [
            ("bash", Some(Kind::Bash)),
            ("/usr/bin/bash", Some(Kind::Bash)),
            (r"C:\Program Files\Git\bin\bash.exe", Some(Kind::Bash)),
            ("C:/Program Files/Git/bin/BASH.EXE", Some(Kind::Bash)),
            ("/bin/zsh", Some(Kind::Zsh)),
            ("fish", Some(Kind::Fish)),
            ("pwsh", Some(Kind::PowerShell)),
            ("pwsh.exe", Some(Kind::PowerShell)),
            (
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
                Some(Kind::PowerShell),
            ),
            ("PowerShell.exe", Some(Kind::PowerShell)),
            ("cmd.exe", None),
            ("nu", None),
            ("rbash", None),
            ("bash.sh", None),
            ("/opt/zsh/bin/sh", None),
            ("", None),
        ] {
            assert_eq!(shell_of(program), shell, "{program:?}");
        }
    }

    #[test]
    fn a_program_that_is_no_known_shell_is_left_exactly_as_it_was() {
        for program in ["cmd.exe", "nu", "/bin/sh", "vim"] {
            let before = CommandSpec::new(program).arg("-x").env("A", "b");
            let mut after = before.clone();
            hook_shell(&mut after, program, &scripts(), false, &nothing);
            assert_eq!(after, before, "{program}");
        }
    }

    #[test]
    fn bash_is_given_one_line_to_run_before_its_first_prompt() {
        let command = hooked("/bin/bash", &["--login"]);
        // Its arguments are its own.
        assert_eq!(words(&command), ["--login"]);
        assert_eq!(
            value(&command, PROMPT_COMMAND).as_deref(),
            Some(BASH_BOOTSTRAP)
        );
        assert_eq!(value(&command, ENABLED).as_deref(), Some("1"));
        assert_eq!(
            value(&command, SCRIPTS),
            Some(scripts().to_string_lossy().into_owned())
        );
        assert!(command.env_remove.is_empty());
    }

    #[test]
    fn what_bash_already_runs_before_a_prompt_still_runs_after_the_line() {
        // What the session sets for itself.
        let mut command = CommandSpec::new("bash").env(PROMPT_COMMAND, "history -a");
        hook_shell(&mut command, "bash", &scripts(), false, &nothing);
        assert_eq!(
            value(&command, PROMPT_COMMAND),
            Some(format!("{BASH_BOOTSTRAP}\nhistory -a"))
        );
        assert_eq!(
            command
                .env
                .iter()
                .filter(|(name, _)| name == PROMPT_COMMAND)
                .count(),
            1
        );

        // What this program was itself started with.
        let mut command = CommandSpec::new("bash");
        hook_shell(&mut command, "bash", &scripts(), false, &|name| {
            (name == PROMPT_COMMAND).then(|| "date".to_owned())
        });
        assert_eq!(
            value(&command, PROMPT_COMMAND),
            Some(format!("{BASH_BOOTSTRAP}\ndate"))
        );
    }

    #[test]
    fn the_line_bash_is_given_is_the_line_its_file_takes_out() {
        let bootstrap = FILES
            .iter()
            .find(|(path, _)| *path == "bash/bootstrap.bash")
            .unwrap()
            .1;
        assert!(!BASH_BOOTSTRAP.contains('\''));
        assert!(
            bootstrap.contains(&format!("line='{BASH_BOOTSTRAP}'")),
            "the bootstrap file does not hold the line it is read by"
        );
        // A zsh that was handed the line as well drops it, and knows it by the same
        // letters.
        let zshenv = FILES
            .iter()
            .find(|(path, _)| *path == "zsh/.zshenv")
            .unwrap()
            .1;
        assert!(zshenv.contains(&format!("== '{BASH_BOOTSTRAP}' ]]")));
    }

    #[test]
    fn zsh_is_pointed_at_our_folder_and_told_the_users() {
        let ours = scripts().join("zsh").to_string_lossy().into_owned();

        let command = hooked("zsh", &[]);
        assert!(command.args.is_empty());
        assert_eq!(value(&command, ZDOTDIR), Some(ours.clone()));
        assert_eq!(value(&command, ZSH_USER_DIR), None);
        // One left over from elsewhere does not reach the shell.
        assert_eq!(command.env_remove, [OsString::from(ZSH_USER_DIR)]);
        assert_eq!(value(&command, ENABLED).as_deref(), Some("1"));

        let mut command = CommandSpec::new("zsh");
        hook_shell(&mut command, "zsh", &scripts(), false, &|name| {
            (name == ZDOTDIR).then(|| "/home/me/.config/zsh".to_owned())
        });
        assert_eq!(value(&command, ZDOTDIR), Some(ours.clone()));
        assert_eq!(
            value(&command, ZSH_USER_DIR).as_deref(),
            Some("/home/me/.config/zsh")
        );
        assert!(command.env_remove.is_empty());

        // The session's own setting is the user's, over this program's.
        let mut command = CommandSpec::new("zsh").env(ZDOTDIR, "/srv/zsh");
        hook_shell(&mut command, "zsh", &scripts(), false, &|_| {
            Some("/elsewhere".to_owned())
        });
        assert_eq!(value(&command, ZDOTDIR), Some(ours));
        assert_eq!(value(&command, ZSH_USER_DIR).as_deref(), Some("/srv/zsh"));
    }

    #[test]
    fn fish_finds_our_folder_first_and_keeps_the_folders_it_had() {
        let ours = scripts().join("fish-data").to_string_lossy().into_owned();

        let mut command = CommandSpec::new("fish");
        hook_shell(&mut command, "fish", &scripts(), false, &nothing);
        assert_eq!(
            value(&command, XDG_DATA_DIRS),
            Some(format!("{ours}:/usr/local/share:/usr/share"))
        );
        assert_eq!(value(&command, FISH_USER_DATA_DIRS), None);
        assert_eq!(command.env_remove, [OsString::from(FISH_USER_DATA_DIRS)]);
        assert_eq!(value(&command, ENABLED).as_deref(), Some("1"));

        let mut command = CommandSpec::new("fish");
        hook_shell(&mut command, "fish", &scripts(), false, &|name| {
            (name == XDG_DATA_DIRS).then(|| "/opt/share:/usr/share".to_owned())
        });
        assert_eq!(
            value(&command, XDG_DATA_DIRS),
            Some(format!("{ours}:/opt/share:/usr/share"))
        );
        assert_eq!(
            value(&command, FISH_USER_DATA_DIRS).as_deref(),
            Some("/opt/share:/usr/share")
        );

        // On Windows the list cannot hold a path, so fish is left alone there.
        let before = CommandSpec::new("fish.exe");
        let mut after = before.clone();
        hook_shell(&mut after, "fish.exe", &scripts(), true, &nothing);
        assert_eq!(after, before);
    }

    #[test]
    fn powershell_is_asked_to_load_the_script_and_stay() {
        for program in ["pwsh", "powershell.exe"] {
            let command = hooked(program, &["-NoLogo"]);
            assert_eq!(
                words(&command),
                ["-NoLogo", "-NoExit", "-Command", POWERSHELL_PREAMBLE]
            );
            assert_eq!(value(&command, ENABLED).as_deref(), Some("1"));
            assert_eq!(
                value(&command, SCRIPTS),
                Some(scripts().to_string_lossy().into_owned())
            );
        }
        // A command line would have to escape these, each in its own way.
        assert!(!POWERSHELL_PREAMBLE.contains('"'));
        assert!(POWERSHELL_PREAMBLE.is_ascii());
    }

    #[test]
    fn powershell_with_something_of_its_own_to_run_is_left_alone() {
        let own: [&[&str]; 16] = [
            &["-Command", "Get-Date"],
            &["-c", "Get-Date"],
            &["-NoLogo", "-com", "Get-Date"],
            &["/Command", "Get-Date"],
            &["-File", "script.ps1"],
            &["-f", "script.ps1"],
            &["-EncodedCommand", "RwBlAHQALQBEAGEAdABlAA=="],
            &["-e", "RwBlAHQALQBEAGEAdABlAA=="],
            &["-ec", "RwBlAHQALQBEAGEAdABlAA=="],
            &["-enc", "RwBlAHQALQBEAGEAdABlAA=="],
            &["-CommandWithArgs", "$args", "a"],
            &["script.ps1"],
            &["-NoProfile", "script.ps1"],
            &["-Version"],
            &["-NonInteractive"],
            &["-"],
        ];
        for args in own {
            let before = CommandSpec::new("pwsh").args(args.iter().copied());
            let mut after = before.clone();
            hook_shell(&mut after, "pwsh", &scripts(), false, &nothing);
            assert_eq!(after, before, "{args:?}");
        }
        // A switch whose value is missing is PowerShell's to complain about.
        assert!(!powershell_takes_preamble(&["-ExecutionPolicy".into()]));
    }

    #[test]
    fn powershell_switches_that_leave_it_at_a_prompt_do_not_stand_in_the_way() {
        let fine: [&[&str]; 8] = [
            &[],
            &["-NoLogo", "-NoProfile"],
            &["-nol", "-nop", "-noe"],
            &["-ExecutionPolicy", "Bypass", "-NoLogo"],
            &["-ep", "RemoteSigned"],
            &["-WorkingDirectory", "~"],
            &["-wd", "C:/work", "-l"],
            &["--nologo", "/NoProfile", "-WindowStyle", "Normal"],
        ];
        for args in fine {
            let args: Vec<OsString> = args.iter().map(Into::into).collect();
            assert!(powershell_takes_preamble(&args), "{args:?}");
        }
    }

    #[test]
    fn a_distribution_gets_what_bash_and_zsh_need_and_a_list_of_it() {
        let mut env = vec![(OsString::from("A"), OsString::from("b"))];
        let entries = hook_distro(&mut env, &scripts(), None);
        assert_eq!(
            entries,
            [
                "LUME_SHELL_INTEGRATION",
                "LUME_SHELL_INTEGRATION_DIR/p",
                "PROMPT_COMMAND",
                "ZDOTDIR/p",
            ]
        );
        let value = |name: &str| {
            env.iter()
                .find(|(set, _)| set == name)
                .map(|(_, value)| value.to_string_lossy().into_owned())
        };
        assert_eq!(value("A").as_deref(), Some("b"));
        assert_eq!(value(ENABLED).as_deref(), Some("1"));
        assert_eq!(
            value(SCRIPTS),
            Some(scripts().to_string_lossy().into_owned())
        );
        assert_eq!(value(PROMPT_COMMAND).as_deref(), Some(BASH_BOOTSTRAP));
        assert_eq!(
            value(ZDOTDIR),
            Some(scripts().join("zsh").to_string_lossy().into_owned())
        );
    }

    #[test]
    fn a_variable_the_user_sends_into_the_distribution_stays_theirs() {
        // Listed by the user: ours would cross as theirs does, untranslated.
        let mut env = Vec::new();
        let entries = hook_distro(
            &mut env,
            &scripts(),
            Some("GOPATH/p:ZDOTDIR:PROMPT_COMMAND/u"),
        );
        assert_eq!(
            entries,
            ["LUME_SHELL_INTEGRATION", "LUME_SHELL_INTEGRATION_DIR/p"]
        );
        assert!(
            !env.iter()
                .any(|(name, _)| name == ZDOTDIR || name == PROMPT_COMMAND)
        );

        // Set by the session: a folder of the distribution, which is not ours to move;
        // and a command to run before prompts, which still runs after the line.
        let mut env = vec![
            (OsString::from(ZDOTDIR), OsString::from("/home/me/zsh")),
            (OsString::from(PROMPT_COMMAND), OsString::from("history -a")),
        ];
        let entries = hook_distro(&mut env, &scripts(), None);
        assert_eq!(
            entries,
            [
                "LUME_SHELL_INTEGRATION",
                "LUME_SHELL_INTEGRATION_DIR/p",
                "PROMPT_COMMAND",
            ]
        );
        let value = |name: &str| {
            env.iter()
                .find(|(set, _)| set == name)
                .map(|(_, value)| value.to_string_lossy().into_owned())
        };
        assert_eq!(value(ZDOTDIR).as_deref(), Some("/home/me/zsh"));
        assert_eq!(
            value(PROMPT_COMMAND),
            Some(format!("{BASH_BOOTSTRAP}\nhistory -a"))
        );
    }

    /// A folder of this test's own under the system's temporary folder.
    pub(super) fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lume-integration-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_scripts_are_written_once_under_a_name_for_what_they_hold() {
        let root = scratch("install");
        let folder = install(&root).unwrap();
        assert_eq!(folder.parent(), Some(root.as_path()));
        let name = folder.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name.len(), 16);
        assert!(name.bytes().all(|b| b.is_ascii_hexdigit()));

        for (path, text) in FILES {
            let file = path
                .split('/')
                .fold(folder.clone(), |at, part| at.join(part));
            let there = std::fs::read(&file).unwrap();
            assert_eq!(there, written(text).as_bytes(), "{path}");
            // Shells on Linux read these as they are.
            assert!(!there.contains(&b'\r'), "{path}");
        }
        assert!(folder.join("zsh").join(".zshenv").is_file());

        // Asked again, it is the same folder and nothing is written.
        let modified = |file: &str| {
            std::fs::metadata(folder.join(file))
                .unwrap()
                .modified()
                .unwrap()
        };
        let before = modified("lume.bash");
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(install(&root).unwrap(), folder);
        assert_eq!(modified("lume.bash"), before);

        // A file that was changed or lost is written again.
        std::fs::write(folder.join("lume.zsh"), "broken").unwrap();
        std::fs::remove_file(folder.join("lume.ps1")).unwrap();
        install(&root).unwrap();
        for file in ["lume.zsh", "lume.ps1"] {
            let text = FILES.iter().find(|(path, _)| *path == file).unwrap().1;
            assert_eq!(
                std::fs::read(folder.join(file)).unwrap(),
                written(text).as_bytes()
            );
        }
        // Nothing half-written is left beside them.
        let stray = std::fs::read_dir(&folder)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".partial"));
        assert!(!stray);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_folders_name_follows_the_text_and_not_its_line_endings() {
        let unix = [("a", "one\ntwo\n".to_owned())];
        let windows = [("a", written("one\r\ntwo\r\n"))];
        assert_eq!(content_name(&unix), content_name(&windows));
        assert_ne!(
            content_name(&unix),
            content_name(&[("a", "one\n".to_owned())])
        );
        assert_ne!(
            content_name(&unix),
            content_name(&[("b", "one\ntwo\n".to_owned())])
        );
        // Where one file ends and the next begins is part of what is named.
        assert_ne!(
            content_name(&[("a", "bc".to_owned())]),
            content_name(&[("ab", "c".to_owned())])
        );
    }

    #[test]
    fn the_script_for_powershell_is_plain_ascii() {
        // Windows PowerShell reads a file without a mark of its encoding as the
        // system's own code page, so nothing else would come through whole.
        let script = FILES
            .iter()
            .find(|(path, _)| *path == "lume.ps1")
            .unwrap()
            .1;
        assert!(script.is_ascii());
    }
}

/// The scripts in shells that are really there. Each test starts a shell in a terminal
/// of its own, types into it and reads what it wrote. They are not run with the
/// others: which shells a machine has is not the tests' to decide, and none of them
/// may write to a history that is somebody's own. Run one with `--ignored`.
#[cfg(test)]
mod real_shells {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use lume_proto::{Launcher, SpawnSpec, TermSize};
    use lume_pty::{ChildKiller, CommandSpec, Pty, PtySize};

    use super::tests::scratch;
    use super::*;
    use crate::launch::{Launch, launch};

    /// How long a shell has to print what a step waits for.
    const STEP: Duration = Duration::from_secs(30);

    const ESC: &str = "\u{1b}";

    /// A shell in a terminal, and everything it has written so far.
    struct Running {
        _pty: Box<dyn Pty>,
        writer: Box<dyn Write + Send>,
        killer: Box<dyn ChildKiller>,
        output: Arc<Mutex<Vec<u8>>>,
    }

    impl Running {
        fn start(command: &CommandSpec) -> Self {
            let (mut pty, child) = lume_pty::native()
                .spawn(command, PtySize::cells(120, 30))
                .expect("the shell could not be started");
            let mut reader = pty.take_reader().unwrap();
            let writer = pty.take_writer().unwrap();
            let killer = child.killer();
            let output = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&output);
            std::thread::spawn(move || {
                let mut child = child;
                let mut buf = vec![0u8; 16 * 1024];
                while let Ok(read) = reader.read(&mut buf) {
                    if read == 0 {
                        break;
                    }
                    sink.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .extend_from_slice(&buf[..read]);
                }
                let _ = child.wait();
            });
            Self {
                _pty: pty,
                writer,
                killer,
                output,
            }
        }

        fn text(&self) -> String {
            let output = self.output.lock().unwrap_or_else(PoisonError::into_inner);
            String::from_utf8_lossy(&output).into_owned()
        }

        /// How much has been written so far: a place to look from later.
        fn mark(&self) -> usize {
            self.text().len()
        }

        fn enter(&mut self, line: &str) {
            self.writer.write_all(line.as_bytes()).unwrap();
            self.writer.write_all(b"\r").unwrap();
            self.writer.flush().unwrap();
        }

        /// Waits until `needle` has been written at or after `from`, and says where
        /// it ends.
        fn expect(&self, from: usize, needle: &str) -> usize {
            let deadline = Instant::now() + STEP;
            loop {
                let text = self.text();
                if let Some(at) = text.get(from..).and_then(|rest| rest.find(needle)) {
                    return from + at + needle.len();
                }
                assert!(
                    Instant::now() < deadline,
                    "the shell did not write {needle:?}; since the mark it wrote {:?}",
                    text.get(from..).unwrap_or_default()
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        /// Gives the shell a moment and then reads what it wrote since `from`.
        fn settled(&self, from: usize) -> String {
            std::thread::sleep(Duration::from_millis(700));
            self.text().get(from..).unwrap_or_default().to_owned()
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            // With LUME_TEST_SHOW set, everything the shell wrote is printed, for
            // reading the sequences by eye.
            if std::env::var_os("LUME_TEST_SHOW").is_some() {
                eprintln!("{:?}", self.text());
            }
            let _ = self.killer.kill();
        }
    }

    fn osc(body: &str) -> String {
        format!("{ESC}]{body}{ESC}\\")
    }

    /// A folder whose name has a blank and a letter outside ASCII, and its name.
    fn odd_folder(test: &str) -> (PathBuf, &'static str) {
        const NAME: &str = "lume si \u{e9}";
        let dir = std::env::temp_dir()
            .join(format!("lume-integration-{test}-{}", std::process::id()))
            .join(NAME);
        std::fs::create_dir_all(&dir).unwrap();
        (dir, "lume%20si%20%C3%A9")
    }

    fn spec(launcher: Launcher, env: &[(&str, &str)]) -> SpawnSpec {
        SpawnSpec {
            launcher,
            cwd: None,
            env: env
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            size: TermSize::new(120, 30).unwrap(),
            shell_integration: true,
            scrollback_lines: None,
        }
    }

    /// What every shell is put through from its first prompt on: a folder with an odd
    /// name, a command that fails, and a line with nothing on it. `cd` is the line
    /// that goes to the folder, whose report must end in `encoded_folder`; `fail` is a
    /// line that fails and the exit code it fails with.
    fn exercise(shell: &mut Running, cd: &str, fail: (&str, &str), encoded_folder: &str) {
        // The first prompt: where the shell is, and that a prompt begins. Nothing has
        // been run, so nothing is said to have ended.
        let prompt = shell.expect(0, &osc("133;A"));
        let before = shell.text();
        assert!(
            before[..prompt].contains(&format!("{ESC}]7;file://")),
            "{before:?}"
        );
        assert!(!before[..prompt].contains("133;D"), "{before:?}");
        shell.expect(prompt, &osc("133;B"));

        // A folder with a blank and a letter outside ASCII in its name.
        let from = shell.mark();
        shell.enter(cd);
        let started = shell.expect(from, &format!("{ESC}]133;C"));
        let ended = shell.expect(started, &osc("133;D;0"));
        let reported = shell.expect(ended, &format!("{ESC}]7;file://"));
        let rest = shell.text();
        let url_end = rest[reported..].find(ESC).unwrap() + reported;
        let url = &rest[reported..url_end];
        assert!(url.ends_with(encoded_folder), "{url:?}");
        assert!(url.is_ascii() && !url.contains(' '), "{url:?}");
        shell.expect(url_end, &osc("133;A"));

        // A command that fails says how.
        let from = shell.mark();
        shell.enter(fail.0);
        let started = shell.expect(from, &format!("{ESC}]133;C"));
        shell.expect(started, &osc(&format!("133;D;{}", fail.1)));

        // A line with nothing on it runs nothing: a new prompt, and no end of a
        // command.
        std::thread::sleep(Duration::from_millis(300));
        let from = shell.mark();
        shell.enter("");
        shell.expect(from, &osc("133;A"));
        let after = shell.settled(from);
        assert!(!after.contains("133;D"), "{after:?}");
        assert!(!after.contains("133;C"), "{after:?}");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "starts Git Bash"]
    fn git_bash_reports_through_its_prompt_command() {
        let root = scratch("git-bash");
        let scripts = install(&root).unwrap();
        let (folder, encoded) = odd_folder("git-bash");
        let asked = spec(
            Launcher::Shell {
                program: r"C:\Program Files\Git\bin\bash.exe".to_owned(),
                args: vec!["--norc".to_owned(), "--noprofile".to_owned()],
                login: false,
            },
            // Nothing typed here is kept, and the prompt is one a user might have.
            &[("HISTFILE", "/dev/null"), ("PS1", "[\\w]\\$ ")],
        );
        let Launch { command, .. } = launch(asked, Some(&scripts)).unwrap();
        let mut shell = Running::start(&command);

        let cd = format!("cd '{}'", folder.to_string_lossy().replace('\\', "/"));
        exercise(&mut shell, &cd, ("(exit 3)", "3"), encoded);

        // The command line is told, the folder is spelt with its drive, and what the
        // user had before a prompt and as a prompt is still there.
        let from = shell.mark();
        shell.enter("echo \"a b\"; true");
        shell.expect(from, &osc("133;C;cmdline_url=echo%20%22a%20b%22%3B%20true"));
        let from = shell.mark();
        shell.enter("echo \"$PROMPT_COMMAND\" | tr '\\n' '+'; echo \"<$PS0>\"");
        shell.expect(from, "__lume_precmd+__lume_mark_prompt+");
        let text = shell.text();
        let url_at = text.rfind(&format!("{ESC}]7;file://")).unwrap();
        let url = &text[url_at..];
        assert!(
            url.split('/')
                .nth(3)
                .is_some_and(|drive| drive.len() == 2 && drive.ends_with(':')),
            "{:?}",
            &url[..url.len().min(120)]
        );
        // The variable that carried the line in is not handed on to a shell started
        // from this one.
        let from = shell.mark();
        shell.enter("bash --norc --noprofile -c 'echo \"inner=[${PROMPT_COMMAND-}]\"'");
        shell.expect(from, "inner=[]");
        shell.enter("exit");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A shell of a Unix machine with a home folder of its own, so that it reads no
    /// start-up file and keeps no history that is somebody's. `rc` names a start-up
    /// file to make there and what to put in it.
    #[cfg(unix)]
    fn unix_shell(
        test: &str,
        program: &str,
        args: &[&str],
        rc: (&str, &str),
    ) -> (Running, PathBuf) {
        let root = scratch(test);
        let scripts = install(&root).unwrap();
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(rc.0), rc.1).unwrap();
        let home = home.to_string_lossy().into_owned();
        let asked = spec(
            Launcher::Shell {
                program: program.to_owned(),
                args: args.iter().map(|&arg| arg.to_owned()).collect(),
                login: false,
            },
            &[("HOME", &home), ("HISTFILE", "/dev/null")],
        );
        let Launch { command, .. } = launch(asked, Some(&scripts)).unwrap();
        (Running::start(&command), root)
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "starts bash"]
    fn bash_reports_through_its_prompt_command() {
        let (folder, encoded) = odd_folder("bash");
        let (mut shell, root) =
            unix_shell("bash", "bash", &["--noprofile", "--norc"], (".bashrc", ""));
        let cd = format!("cd '{}'", folder.to_string_lossy());
        exercise(&mut shell, &cd, ("(exit 3)", "3"), encoded);
        shell.enter("exit");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "starts zsh"]
    fn zsh_reports_through_its_start_up_folder() {
        let (folder, encoded) = odd_folder("zsh");
        // A start-up file of the user's own, which must still be read: it sets the
        // prompt, and without any such file zsh would stop to ask a new user questions.
        let (mut shell, root) = unix_shell("zsh", "zsh", &["-i"], (".zshrc", "PS1='own%# '\n"));
        let cd = format!("cd '{}'", folder.to_string_lossy());
        exercise(&mut shell, &cd, ("(exit 3)", "3"), encoded);
        let from = shell.mark();
        shell.enter("print -r -- \"zdotdir=[${ZDOTDIR-}] prompt=[${PS1[(i)own]}]\"");
        // The folder zsh was pointed at is forgotten, and the prompt drawn is the
        // user's with the marks around it.
        shell.expect(from, "zdotdir=[]");
        shell.enter("exit");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Git Bash reading `rc` as its start-up file, with or without the hook.
    #[cfg(windows)]
    fn git_bash_with(test: &str, rc: &str, hooked: bool) -> (Running, PathBuf) {
        let root = scratch(test);
        let scripts = install(&root).unwrap();
        let file = root.join("user.bashrc");
        let rc = rc.replace("@SCRIPTS@", &scripts.to_string_lossy().replace('\\', "/"));
        std::fs::write(&file, rc).unwrap();
        let asked = spec(
            Launcher::Shell {
                program: r"C:\Program Files\Git\bin\bash.exe".to_owned(),
                args: vec![
                    "--noprofile".to_owned(),
                    "--rcfile".to_owned(),
                    file.to_string_lossy().replace('\\', "/"),
                ],
                login: false,
            },
            &[("HISTFILE", "/dev/null")],
        );
        let Launch { command, .. } = launch(asked, hooked.then_some(scripts.as_path())).unwrap();
        (Running::start(&command), root)
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "starts Git Bash"]
    fn bash_keeps_what_the_user_set_up_around_the_hook() {
        // A start-up file that sets a prompt from the exit code before every prompt,
        // puts that in front of whatever was there, has an alias, and is strict about
        // variables that are not set.
        let rc = "alias greet='echo alias-works'\n\
                  user_prompt() { PS1=\"{$?}\\$ \"; }\n\
                  PROMPT_COMMAND=\"user_prompt; $PROMPT_COMMAND\"\n\
                  set -u\n";
        let (mut shell, root) = git_bash_with("bash-user", rc, true);
        shell.expect(0, "{0}$ ");
        let from = shell.mark();
        shell.enter("(exit 3)");
        let ended = shell.expect(from, &osc("133;D;3"));
        // The user's function saw the exit code, and its prompt is the one marked.
        shell.expect(ended, &format!("{}{{3}}$ {}", osc("133;A"), osc("133;B")));
        let from = shell.mark();
        shell.enter("greet");
        shell.expect(from, "alias-works");
        let from = shell.mark();
        shell.enter("echo \"[$PROMPT_COMMAND]\" | tr '\\n' '+'");
        shell.expect(from, "[__lume_precmd+user_prompt; :+__lume_mark_prompt]");
        let text = shell.text();
        for wrong in [
            "unbound variable",
            "command not found",
            "syntax error",
            "No such file",
        ] {
            assert!(!text.contains(wrong), "{wrong:?} in {text:?}");
        }
        shell.enter("exit");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "starts Git Bash"]
    fn bash_that_sets_its_own_prompt_command_simply_reports_nothing() {
        let rc = "PROMPT_COMMAND='true'\nPS1='plain$ '\n";
        let (mut shell, root) = git_bash_with("bash-own", rc, true);
        shell.expect(0, "plain$ ");
        let from = shell.mark();
        shell.enter("echo status=$?");
        shell.expect(from, "status=0");
        shell.expect(from, "plain$ ");
        let text = shell.settled(0);
        assert!(!text.contains("]133;"), "{text:?}");
        for wrong in ["command not found", "syntax error", "No such file"] {
            assert!(!text.contains(wrong), "{wrong:?} in {text:?}");
        }
        shell.enter("exit");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "starts Git Bash"]
    fn bash_with_a_list_of_prompt_commands_keeps_its_list() {
        let rc = "note() { noted=yes; }\n\
                  PROMPT_COMMAND=(note \"$PROMPT_COMMAND\")\n\
                  PS1='list$ '\n";
        let (mut shell, root) = git_bash_with("bash-list", rc, true);
        shell.expect(0, &format!("{}list$ ", osc("133;A")));
        let from = shell.mark();
        shell.enter("(exit 4)");
        shell.expect(from, &osc("133;D;4"));
        let from = shell.mark();
        shell.enter(
            "echo \"${#PROMPT_COMMAND[@]}:${PROMPT_COMMAND[0]}:${PROMPT_COMMAND[1]}:$noted\"",
        );
        shell.expect(from, "4:__lume_precmd:note:yes");
        shell.enter("exit");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "starts Git Bash"]
    fn the_script_read_from_a_start_up_file_works_without_the_hook_and_once_with_it() {
        let rc = "PS1='own$ '\n. '@SCRIPTS@/lume.bash'\n";
        for hooked in [false, true] {
            let (mut shell, root) = git_bash_with("bash-manual", rc, hooked);
            shell.expect(0, &format!("{}own$ {}", osc("133;A"), osc("133;B")));
            let from = shell.mark();
            shell.enter("(exit 5)");
            shell.expect(from, &osc("133;D;5"));
            let after = shell.settled(from);
            // Once, also when it was both read by the file and asked for by the hook.
            assert_eq!(
                after.matches("133;D;").count(),
                1,
                "hooked {hooked}: {after:?}"
            );
            assert_eq!(
                after.matches("133;C").count(),
                1,
                "hooked {hooked}: {after:?}"
            );
            let from = shell.mark();
            shell.enter("echo \"[$PROMPT_COMMAND]\" | tr '\\n' '+'");
            let expected = if hooked {
                "[__lume_precmd+:+__lume_mark_prompt]"
            } else {
                "[__lume_precmd+__lume_mark_prompt]"
            };
            shell.expect(from, expected);
            shell.enter("exit");
            drop(shell);
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "starts Git Bash"]
    fn bash_says_which_command_line_runs_as_far_as_its_history_knows() {
        let rc = "PS1='h$ '\nHISTCONTROL=ignoreboth\n";
        let (mut shell, root) = git_bash_with("bash-history", rc, true);
        shell.expect(0, "h$ ");
        // A line over several lines is one command line.
        let from = shell.mark();
        shell.enter("echo one; \\");
        shell.enter("echo two");
        shell.expect(from, &osc("133;C;cmdline_url=echo%20one%3B%20echo%20two"));
        shell.expect(from, &osc("133;D;0"));
        // With the history switched off there is no line to tell, and none is made up.
        let from = shell.mark();
        shell.enter("set +o history");
        shell.expect(from, &osc("133;D;0"));
        let from = shell.mark();
        shell.enter("echo quiet");
        shell.expect(from, &osc("133;C"));
        shell.expect(from, &osc("133;D;0"));
        shell.enter("exit");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "starts Windows PowerShell"]
    fn windows_powershell_reports_through_its_prompt() {
        let root = scratch("powershell");
        let scripts = install(&root).unwrap();
        let (folder, encoded) = odd_folder("powershell");
        // The arguments are not the ones a session gets: one more command goes in
        // front, so that nothing typed here is written to the user's history file.
        let quiet =
            format!("Set-PSReadLineOption -HistorySaveStyle SaveNothing; {POWERSHELL_PREAMBLE}");
        let command = CommandSpec::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", &quiet])
            .env(ENABLED, ENABLED_VALUE)
            .env(SCRIPTS, &scripts);
        let mut shell = Running::start(&command);

        let cd = format!("Set-Location -LiteralPath '{}'", folder.to_string_lossy());
        exercise(&mut shell, &cd, ("cmd /c exit 3", "3"), encoded);

        // A failure of PowerShell's own is a failure, and not with the exit code an
        // earlier program left behind. The prompt the user had is still drawn.
        let from = shell.mark();
        shell.enter("$null = 1/0");
        let started = shell.expect(from, &format!("{ESC}]133;C;cmdline_url=%24null%20"));
        let ended = shell.expect(started, &osc("133;D;1"));
        shell.expect(ended, "PS ");
        // The same program failing the same way twice is told both times.
        for _ in 0..2 {
            let from = shell.mark();
            shell.enter("cmd /c exit 3");
            shell.expect(from, &osc("133;D;3"));
        }
        let from = shell.mark();
        shell.enter("Get-Date > $null");
        shell.expect(from, &osc("133;D;0"));
        shell.enter("exit");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Windows PowerShell with `before` run ahead of the line that loads the script,
    /// and nothing typed into it written to the user's history file.
    #[cfg(windows)]
    fn powershell_with(test: &str, before: &str) -> (Running, PathBuf) {
        let root = scratch(test);
        let scripts = install(&root).unwrap();
        let first = format!(
            "Set-PSReadLineOption -HistorySaveStyle SaveNothing; {before}; {POWERSHELL_PREAMBLE}"
        );
        let command = CommandSpec::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", &first])
            .env(ENABLED, ENABLED_VALUE)
            .env(SCRIPTS, &scripts);
        (Running::start(&command), root)
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "starts Windows PowerShell"]
    fn powershell_keeps_the_users_own_prompt_and_what_it_reads() {
        // A prompt of the user's own that shows whether the last command line
        // succeeded and with what exit code, as many prompts do.
        let own = "function prompt { \"[$?|$LASTEXITCODE]> \" }";
        // Loading the script twice is loading it once.
        let twice = format!("{own}; {POWERSHELL_PREAMBLE}");
        let (mut shell, root) = powershell_with("powershell-own", &twice);
        shell.expect(0, &osc("133;A"));
        let from = shell.mark();
        shell.enter("cmd /c exit 3");
        let ended = shell.expect(from, &osc("133;D;3"));
        shell.expect(
            ended,
            &format!("{}[False|3]> {}", osc("133;A"), osc("133;B")),
        );
        let from = shell.mark();
        shell.enter("cmd /c exit 0");
        let ended = shell.expect(from, &osc("133;D;0"));
        shell.expect(
            ended,
            &format!("{}[True|0]> {}", osc("133;A"), osc("133;B")),
        );
        let after = shell.settled(from);
        assert_eq!(after.matches("133;D;").count(), 1, "{after:?}");
        assert_eq!(after.matches("133;A").count(), 1, "{after:?}");
        shell.enter("exit");
        drop(shell);
        let _ = std::fs::remove_dir_all(&root);

        // A prompt that draws itself and returns nothing still gets the two
        // characters the host adds for it.
        let silent = "function prompt { Write-Host 'drawn' -NoNewline }";
        let (mut shell, root) = powershell_with("powershell-silent", silent);
        let drawn = shell.expect(0, &format!("{}drawn", osc("133;A")));
        shell.expect(drawn, &format!("PS>{}", osc("133;B")));
        shell.enter("exit");
        drop(shell);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "starts Windows PowerShell"]
    fn powershell_started_as_a_session_starts_it_marks_its_first_prompt() {
        let root = scratch("powershell-args");
        let scripts = install(&root).unwrap();
        let asked = spec(
            Launcher::Shell {
                program: "powershell.exe".to_owned(),
                args: vec!["-NoLogo".to_owned(), "-NoProfile".to_owned()],
                login: false,
            },
            &[],
        );
        let Launch { command, .. } = launch(asked, Some(&scripts)).unwrap();
        // Nothing is typed: the line editor would write it to the user's history.
        let shell = Running::start(&command);
        let prompt = shell.expect(0, &osc("133;A"));
        shell.expect(prompt, &osc("133;B"));
        assert!(shell.text().contains(&format!("{ESC}]7;file://")));
        drop(shell);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "starts bash in the WSL distribution named by LUME_TEST_DISTRO"]
    fn bash_in_a_distribution_reports_through_its_prompt_command() {
        let distro = std::env::var("LUME_TEST_DISTRO").unwrap_or_else(|_| "Ubuntu".to_owned());
        let root = scratch("wsl");
        let scripts = install(&root).unwrap();
        let asked = spec(
            Launcher::Wsl { distro, user: None },
            &[("HISTFILE", "/dev/null"), ("PS1", "[\\w]\\$ ")],
        );
        let Launch { mut command, .. } = launch(asked, Some(&scripts)).unwrap();
        // Not the user's own shell with the user's own files: a bash that reads none.
        command
            .args
            .extend(["--exec", "bash", "--norc", "--noprofile"].map(Into::into));
        let mut shell = Running::start(&command);

        exercise(
            &mut shell,
            "mkdir -p '/tmp/lume si \u{e9}' && cd '/tmp/lume si \u{e9}'",
            ("(exit 3)", "3"),
            "/tmp/lume%20si%20%C3%A9",
        );
        let from = shell.mark();
        // Both folders crossed over spelt as the distribution spells them. The one
        // meant for a zsh is no longer set, so a zsh started from here reads the
        // user's own files.
        shell.enter(
            "cd /; rmdir '/tmp/lume si \u{e9}'; echo \"dir=$LUME_SHELL_INTEGRATION_DIR now=[${ZDOTDIR-}]\"; \
             tr '\\0' '\\n' < /proc/$$/environ | grep '^ZDOTDIR='",
        );
        let dir = shell.expect(from, "dir=/mnt/");
        shell.expect(dir, "now=[]");
        shell.expect(dir, "ZDOTDIR=/mnt/");
        shell.expect(dir, "/zsh\r\n");
        shell.enter("exit");
        let _ = std::fs::remove_dir_all(&root);
    }
}
