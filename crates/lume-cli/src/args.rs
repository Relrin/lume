//! The command line of `lume`.

use std::time::Duration;

use lume_config::Platform;
use lume_proto::app::{AppRequestBody, Direction, OpenSpec, SpawnPlace};
use lume_proto::{Launcher, SessionId, TermSize};

/// What the arguments may be, shown on request and when they are not understood.
pub const USAGE: &str = "\
usage: lume [--endpoint <name>] [--workspace <name>] [-e <program> [<arg>...]]
       lume ctl [--endpoint <name>] [--timeout <seconds>] <command>
       lume config schema | path | check [<file>]
       lume keymap [--platform windows|macos|linux] [--workspace <name>] [--json]
       lume theme import <file> [--name <name>]
       lume shell-integration bash|zsh|fish|powershell
       lume --help | --version

Without a command, lume starts the application, or brings its window to the front if
it is running. --workspace shows that workspace, beginning it if there is none of that
name. -e runs a program in a new tab, started in the directory lume was typed in;
everything after the program is the program's own.

Commands of lume ctl, which drive the running application:
  status                         what the application is and how far its start has come
  list                           every session, and the pane that shows it
  spawn [--tab | --split left|right|up|down | --window] [--profile <id>]
        [--cwd <dir>] [--size <cols>x<rows>] [--env <name>=<value>]...
        [-- <program> [<arg>...]]
                                 start a session in a new tab, in a new pane beside
                                 the focused one, or in a new window; --profile says
                                 which profile it runs, -- a program instead
  send [<target>] [--enter] [--escapes] <text>...
                                 type text into a terminal; --enter ends it with a
                                 carriage return, --escapes reads \\r \\n \\t \\e \\\\ \\xHH
  wait --text <text> [<target>]  wait until the text is on the terminal's screen
  wait --exit [<target>]         wait until the terminal's program has ended
  capture [<target>] [--plain]   read the terminal's screen; --plain prints its lines
  resize [<target>] <cols>x<rows>
                                 give the terminal this size, whatever fits its pane
  kill <id>                      end a session
  layout view                    print the layout: windows, tabs, splits and panes
  layout apply <intent> | -      change the layout; the intent is JSON, read from
                                 standard input when given as -
  workspaces                     list the workspaces, and say which is on show
  profiles                       list the profiles: the shells, WSL distributions and
                                 SSH hosts a terminal can be started in
  events [<target>]              follow what terminals report: titles, directories,
                                 commands that start and end, the program that has
                                 the terminal, how a session ended. One JSON object a
                                 line, of every session or of the one named, until
                                 interrupted, or for --timeout seconds if that is given
  commands                       list the commands there are
  command <id> [<arguments>]     run a command; its arguments are a JSON object
  quit                           close the application

A <target> is --pane <id> (as in p12) or --session <id>. A command that names neither
means the pane that has the keyboard. Each prints one JSON object. --timeout is how
long a wait may last, in seconds (10 if not given).

Commands on the configuration, which need no application running:
  config schema                  print the JSON Schema of config.toml, for an editor
  config path                    print where config.toml is
  config check [<file>]          say what is wrong in the configuration, or in a file:
                                 unknown keys, values that are not allowed, bindings
                                 and themes that cannot be read
  keymap                         print every key that is bound, the command it runs,
                                 where it holds and where the binding comes from;
                                 --platform lists another platform's keys, --workspace
                                 lays a workspace's file over yours, --json prints JSON
  theme import <file>            make a theme from a Ghostty theme file and put it in
                                 the themes folder; --name names it (the file's name
                                 if not given)
  shell-integration <shell>      print the script that has a shell report its
                                 directory and its commands, for a start-up file of
                                 your own: on a machine reached over SSH, or for a
                                 shell lume does not set up by itself

Exit status: 0 done, 1 refused or failed (for config check: something is wrong), 2
arguments not understood, 3 a wait ran out of time, 4 the application is not running.";

/// How long a wait may last when `--timeout` is not given.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// What `lume` was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    /// Start the application or bring it to the front.
    Start {
        endpoint: Option<String>,
        /// The workspace to show.
        workspace: Option<String>,
        /// A program to run in a new tab, and its arguments. Empty for none.
        exec: Vec<String>,
    },
    /// Send one request to the running application.
    Ctl(Ctl),
    /// Read or write the configuration. The application need not be running.
    Offline(Offline),
}

/// Something done with the configuration's files alone.
#[derive(Debug, PartialEq, Eq)]
pub enum Offline {
    /// Print the JSON Schema of `config.toml`.
    Schema,
    /// Print where `config.toml` is.
    Path,
    /// Say what is wrong in the configuration, or in one file.
    Check { file: Option<String> },
    /// Print the keys that are bound.
    Keymap {
        /// The platform whose keys to print. This one's if not given.
        platform: Option<Platform>,
        /// The workspace whose file is laid over the user's.
        workspace: Option<String>,
        json: bool,
    },
    /// Make a theme from another terminal's colour scheme.
    ThemeImport { file: String, name: Option<String> },
    /// Print the script that has a shell report its directory and its commands.
    ShellIntegration { shell: IntegrationShell },
}

/// A shell there is a reporting script for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegrationShell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

/// One request to the running application.
#[derive(Debug, PartialEq, Eq)]
pub struct Ctl {
    pub endpoint: Option<String>,
    pub request: AppRequestBody,
    /// How long the application may take to answer a request that waits.
    pub timeout: Duration,
    /// How long to follow the sessions, for the request that does: as long as
    /// `--timeout` said, or until interrupted if it was not given.
    pub follow_for: Option<Duration>,
    /// Print a captured screen as its lines instead of as JSON.
    pub plain: bool,
    /// The request's intent is to be read from standard input.
    pub intent_from_input: bool,
}

/// The terminal a request is about, as far as its arguments say.
#[derive(Debug, Default, PartialEq, Eq)]
struct Target {
    pane: Option<String>,
    session: Option<SessionId>,
}

impl Target {
    /// Reads `option` and its value if it is one of the two that name a terminal.
    fn take(&mut self, option: &str, words: &mut Words) -> Result<bool, String> {
        match option {
            "--pane" => {
                let id = value(words, "--pane", "a pane id")?;
                let digits = id.strip_prefix('p').unwrap_or_default();
                if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(format!("{id} is not a pane id (p and a number, as in p12)"));
                }
                self.pane = Some(id);
            }
            "--session" => self.session = Some(session(words, "--session")?),
            _ => return Ok(false),
        }
        if self.pane.is_some() && self.session.is_some() {
            return Err("name a pane or a session, not both".to_owned());
        }
        Ok(true)
    }
}

type Words = std::iter::Peekable<std::vec::IntoIter<String>>;

fn value(words: &mut Words, option: &str, what: &str) -> Result<String, String> {
    words.next().ok_or_else(|| format!("{option} needs {what}"))
}

fn session(words: &mut Words, option: &str) -> Result<SessionId, String> {
    let id = value(words, option, "a session id")?;
    id.parse()
        .map_err(|_| format!("{id} is not a session id (sixteen hexadecimal digits)"))
}

fn size(text: &str) -> Result<TermSize, String> {
    let bad = || format!("{text} is not a size (columns x rows, as in 100x30)");
    let (cols, rows) = text.split_once(['x', 'X']).ok_or_else(bad)?;
    let cols = cols.parse().map_err(|_| bad())?;
    let rows = rows.parse().map_err(|_| bad())?;
    TermSize::new(cols, rows).map_err(|_| bad())
}

fn seconds(text: &str) -> Result<Duration, String> {
    text.parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .map(Duration::from_secs_f64)
        .ok_or_else(|| format!("{text} is not a number of seconds"))
}

/// Reads the escapes `--escapes` allows. The result is text: `\xHH` takes a code below
/// 0x80 only, since a lone byte above it is not a character.
pub fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        let escaped = chars
            .next()
            .ok_or("the text ends in the middle of an escape")?;
        out.push(match escaped {
            'r' => '\r',
            'n' => '\n',
            't' => '\t',
            'e' => '\x1b',
            '0' => '\0',
            '\\' => '\\',
            'x' => {
                let digits: String = chars.by_ref().take(2).collect();
                u8::from_str_radix(&digits, 16)
                    .ok()
                    .filter(|code| digits.len() == 2 && code.is_ascii())
                    .map(char::from)
                    .ok_or_else(|| format!("\\x{digits} is not an escape for a character"))?
            }
            other => return Err(format!("\\{other} is not an escape lume knows")),
        });
    }
    Ok(out)
}

fn timeout_ms(timeout: Duration) -> u64 {
    u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)
}

fn direction(text: &str) -> Result<Direction, String> {
    match text {
        "left" => Ok(Direction::Left),
        "right" => Ok(Direction::Right),
        "up" => Ok(Direction::Up),
        "down" => Ok(Direction::Down),
        other => Err(format!(
            "{other} is not a side to split on (left, right, up or down)"
        )),
    }
}

fn spawn(words: &mut Words) -> Result<AppRequestBody, String> {
    let mut spec = OpenSpec::default();
    let mut place = None;
    let mut placed = |place_given: SpawnPlace| match place.replace(place_given) {
        None => Ok(()),
        Some(_) => Err("spawn takes one of --tab, --split and --window".to_owned()),
    };
    while let Some(word) = words.next() {
        match word.as_str() {
            "--tab" => placed(SpawnPlace::Tab {})?,
            "--split" => placed(SpawnPlace::Split {
                dir: direction(&value(words, "--split", "a side")?)?,
            })?,
            "--window" => placed(SpawnPlace::Window {})?,
            "--cwd" => spec.cwd = Some(value(words, "--cwd", "a directory")?),
            "--profile" => spec.profile = Some(value(words, "--profile", "a profile's id")?),
            "--size" => spec.size = Some(size(&value(words, "--size", "a size")?)?),
            "--env" => {
                let pair = value(words, "--env", "a variable as name=value")?;
                let (name, value) = pair
                    .split_once('=')
                    .filter(|(name, _)| !name.is_empty())
                    .ok_or_else(|| format!("{pair} is not a variable as name=value"))?;
                spec.env.push((name.to_owned(), value.to_owned()));
            }
            "--" => {
                let program = value(words, "--", "a program to run")?;
                spec.launcher = Some(Launcher::Shell {
                    program,
                    args: words.by_ref().collect(),
                    login: false,
                });
            }
            other => return Err(format!("spawn does not take {other}")),
        }
    }
    Ok(AppRequestBody::Spawn {
        spec,
        place: place.unwrap_or_default(),
    })
}

fn send(words: &mut Words) -> Result<AppRequestBody, String> {
    let mut target = Target::default();
    let mut enter = false;
    let mut escapes = false;
    let mut text: Vec<String> = Vec::new();
    while let Some(word) = words.next() {
        if text.is_empty() && target.take(&word, words)? {
            continue;
        }
        match word.as_str() {
            "--enter" if text.is_empty() => enter = true,
            "--escapes" if text.is_empty() => escapes = true,
            // What follows is text, whatever it looks like.
            "--" if text.is_empty() => text.extend(words.by_ref()),
            _ => text.push(word),
        }
    }
    let mut text = text.join(" ");
    if escapes {
        text = unescape(&text)?;
    }
    if enter {
        text.push('\r');
    }
    if text.is_empty() {
        return Err("send needs text to type, or --enter".to_owned());
    }
    Ok(AppRequestBody::Send {
        pane: target.pane,
        session: target.session,
        text,
    })
}

fn wait(words: &mut Words, timeout: Duration) -> Result<AppRequestBody, String> {
    let mut target = Target::default();
    let mut text = None;
    let mut exit = false;
    while let Some(word) = words.next() {
        if target.take(&word, words)? {
            continue;
        }
        match word.as_str() {
            "--text" => text = Some(value(words, "--text", "the text to wait for")?),
            "--exit" => exit = true,
            other => return Err(format!("wait does not take {other}")),
        }
    }
    let timeout_ms = timeout_ms(timeout);
    match (text, exit) {
        (Some(text), false) if !text.is_empty() => Ok(AppRequestBody::WaitText {
            pane: target.pane,
            session: target.session,
            text,
            timeout_ms,
        }),
        (None, true) => Ok(AppRequestBody::WaitExit {
            pane: target.pane,
            session: target.session,
            timeout_ms,
        }),
        _ => Err("wait needs exactly one of --text <text> and --exit".to_owned()),
    }
}

/// Reads text as JSON, saying what it was meant to be when it is not.
fn json(text: &str, what: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|error| format!("{what} is not JSON: {error}"))
}

fn nothing_more(words: &mut Words, command: &str) -> Result<(), String> {
    match words.next() {
        Some(word) => Err(format!("{command} does not take {word}")),
        None => Ok(()),
    }
}

fn ctl(words: &mut Words) -> Result<Ctl, String> {
    let mut endpoint = None;
    let mut timeout = DEFAULT_TIMEOUT;
    let mut follow_for = None;
    let command = loop {
        let word = words.next().ok_or("ctl needs a command; see lume --help")?;
        match word.as_str() {
            "--endpoint" => endpoint = Some(value(words, "--endpoint", "a name")?),
            "--timeout" => {
                timeout = seconds(&value(words, "--timeout", "a number of seconds")?)?;
                follow_for = Some(timeout);
            }
            _ => break word,
        }
    };

    let mut plain = false;
    let mut intent_from_input = false;
    let request = match command.as_str() {
        "status" => {
            nothing_more(words, "status")?;
            AppRequestBody::Status {}
        }
        "list" => {
            nothing_more(words, "list")?;
            AppRequestBody::List {}
        }
        "quit" => {
            nothing_more(words, "quit")?;
            AppRequestBody::Quit {}
        }
        "spawn" => spawn(words)?,
        "send" => send(words)?,
        "wait" => wait(words, timeout)?,
        "capture" => {
            let mut target = Target::default();
            while let Some(word) = words.next() {
                if target.take(&word, words)? {
                    continue;
                }
                match word.as_str() {
                    "--plain" => plain = true,
                    other => return Err(format!("capture does not take {other}")),
                }
            }
            AppRequestBody::Screen {
                pane: target.pane,
                session: target.session,
            }
        }
        "resize" => {
            let mut target = Target::default();
            let mut wanted = None;
            while let Some(word) = words.next() {
                if target.take(&word, words)? {
                    continue;
                }
                match word.as_str() {
                    other if wanted.is_none() => wanted = Some(size(other)?),
                    other => return Err(format!("resize does not take {other}")),
                }
            }
            AppRequestBody::Resize {
                pane: target.pane,
                session: target.session,
                size: wanted.ok_or("resize needs a size, as in 100x30")?,
            }
        }
        "layout" => match words.next().as_deref() {
            Some("view") => {
                nothing_more(words, "layout view")?;
                AppRequestBody::Layout {}
            }
            Some("apply") => {
                let written = value(words, "layout apply", "an intent as JSON, or -")?;
                nothing_more(words, "layout apply")?;
                let intent = if written == "-" {
                    intent_from_input = true;
                    serde_json::Value::Null
                } else {
                    json(&written, "the intent")?
                };
                AppRequestBody::LayoutApply { intent }
            }
            Some(other) => {
                return Err(format!(
                    "{other} is not something ctl layout does (view or apply)"
                ));
            }
            None => return Err("ctl layout needs a command: view or apply".to_owned()),
        },
        "workspaces" => {
            nothing_more(words, "workspaces")?;
            AppRequestBody::Workspaces {}
        }
        "profiles" => {
            nothing_more(words, "profiles")?;
            AppRequestBody::Profiles {}
        }
        "events" => {
            let mut target = Target::default();
            while let Some(word) = words.next() {
                if !target.take(&word, words)? {
                    return Err(format!("events does not take {word}"));
                }
            }
            AppRequestBody::Events {
                pane: target.pane,
                session: target.session,
            }
        }
        "commands" => {
            nothing_more(words, "commands")?;
            AppRequestBody::Commands {}
        }
        "command" => {
            let id = value(words, "command", "a command's id")?;
            let args = match words.next() {
                Some(written) => {
                    let args = json(&written, "the command's arguments")?;
                    if !args.is_object() {
                        return Err("a command's arguments are a JSON object".to_owned());
                    }
                    args
                }
                None => serde_json::Value::Null,
            };
            nothing_more(words, "command")?;
            AppRequestBody::Command {
                id,
                args,
                window: None,
            }
        }
        "kill" => {
            let id = value(words, "kill", "a session id")?;
            nothing_more(words, "kill")?;
            AppRequestBody::Kill {
                session: id
                    .parse()
                    .map_err(|_| format!("{id} is not a session id"))?,
            }
        }
        other => return Err(format!("ctl has no command called {other}")),
    };
    Ok(Ctl {
        endpoint,
        request,
        timeout,
        follow_for,
        plain,
        intent_from_input,
    })
}

fn shell_integration(words: &mut Words) -> Result<Offline, String> {
    const SHELLS: &str = "bash, zsh, fish or powershell";
    let shell = match words.next().as_deref() {
        Some("bash") => IntegrationShell::Bash,
        Some("zsh") => IntegrationShell::Zsh,
        Some("fish") => IntegrationShell::Fish,
        Some("powershell" | "pwsh") => IntegrationShell::PowerShell,
        Some(other) => {
            return Err(format!(
                "{other} is not a shell lume has a script for ({SHELLS})"
            ));
        }
        None => return Err(format!("lume shell-integration needs a shell: {SHELLS}")),
    };
    nothing_more(words, "lume shell-integration")?;
    Ok(Offline::ShellIntegration { shell })
}

fn config(words: &mut Words) -> Result<Offline, String> {
    let command = match words.next().as_deref() {
        Some("schema") => Offline::Schema,
        Some("path") => Offline::Path,
        Some("check") => Offline::Check { file: words.next() },
        Some(other) => {
            return Err(format!(
                "{other} is not something lume config does (schema, path or check)"
            ));
        }
        None => return Err("lume config needs a command: schema, path or check".to_owned()),
    };
    nothing_more(words, "lume config")?;
    Ok(command)
}

fn keymap(words: &mut Words) -> Result<Offline, String> {
    let mut platform = None;
    let mut workspace = None;
    let mut json = false;
    while let Some(word) = words.next() {
        match word.as_str() {
            "--platform" => {
                let name = value(words, "--platform", "windows, macos or linux")?;
                platform = Some(Platform::parse(&name).ok_or_else(|| {
                    format!("{name} is not a platform (windows, macos or linux)")
                })?);
            }
            "--workspace" => workspace = Some(value(words, "--workspace", "a name")?),
            "--json" => json = true,
            other => return Err(format!("lume keymap does not take {other}")),
        }
    }
    Ok(Offline::Keymap {
        platform,
        workspace,
        json,
    })
}

fn theme(words: &mut Words) -> Result<Offline, String> {
    match words.next().as_deref() {
        Some("import") => {}
        Some(other) => {
            return Err(format!("{other} is not something lume theme does (import)"));
        }
        None => return Err("lume theme needs a command: import".to_owned()),
    }
    let mut file = None;
    let mut name = None;
    while let Some(word) = words.next() {
        match word.as_str() {
            "--name" => name = Some(value(words, "--name", "a name")?),
            _ if file.is_none() => file = Some(word),
            other => {
                return Err(format!(
                    "lume theme import takes one file, and {other} is a second"
                ));
            }
        }
    }
    let file = file.ok_or("lume theme import needs the file to import")?;
    Ok(Offline::ThemeImport { file, name })
}

/// Reads the arguments that follow the program's name.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut words: Words = args.into_iter().collect::<Vec<_>>().into_iter().peekable();
    let mut endpoint = None;
    let mut workspace = None;
    let mut exec = Vec::new();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--help" | "-h" | "help" => return Ok(Command::Help),
            "--version" | "-V" => return Ok(Command::Version),
            "--endpoint" => endpoint = Some(value(&mut words, "--endpoint", "a name")?),
            "--workspace" => {
                workspace = Some(value(&mut words, "--workspace", "a workspace's name")?);
            }
            // Everything after this is the program and what it is given, whatever it
            // looks like.
            "-e" | "--exec" => {
                exec = words.by_ref().collect();
                if exec.is_empty() {
                    return Err(format!("{word} needs a program to run"));
                }
            }
            "ctl" => {
                let mut ctl = ctl(&mut words)?;
                // An endpoint named before `ctl` holds unless one is named after it.
                ctl.endpoint = ctl.endpoint.or(endpoint);
                return Ok(Command::Ctl(ctl));
            }
            "config" => return config(&mut words).map(Command::Offline),
            "keymap" => return keymap(&mut words).map(Command::Offline),
            "theme" => return theme(&mut words).map(Command::Offline),
            "shell-integration" => return shell_integration(&mut words).map(Command::Offline),
            other => return Err(format!("{other} is not something lume knows")),
        }
    }
    Ok(Command::Start {
        endpoint,
        workspace,
        exec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    fn request(args: &[&str]) -> AppRequestBody {
        match parsed(args) {
            Ok(Command::Ctl(ctl)) => ctl.request,
            other => panic!("not a request: {other:?}"),
        }
    }

    fn id() -> SessionId {
        "00000000000000ab".parse().unwrap()
    }

    #[test]
    fn no_arguments_start_the_application() {
        let start = |endpoint: Option<&str>, workspace: Option<&str>, exec: &[&str]| {
            Ok(Command::Start {
                endpoint: endpoint.map(str::to_owned),
                workspace: workspace.map(str::to_owned),
                exec: exec.iter().map(|word| (*word).to_owned()).collect(),
            })
        };
        assert_eq!(parsed(&[]), start(None, None, &[]));
        assert_eq!(
            parsed(&["--endpoint", "trial"]),
            start(Some("trial"), None, &[])
        );
        assert_eq!(
            parsed(&["--workspace", "site v2"]),
            start(None, Some("site v2"), &[])
        );
        // What follows the program is the program's, even what lume would understand.
        assert_eq!(
            parsed(&["--workspace", "site", "-e", "htop", "--help", "ctl"]),
            start(None, Some("site"), &["htop", "--help", "ctl"])
        );
        assert!(parsed(&["-e"]).is_err());
        assert!(parsed(&["--workspace"]).is_err());
        assert_eq!(parsed(&["--help"]), Ok(Command::Help));
        assert_eq!(parsed(&["--version"]), Ok(Command::Version));
    }

    #[test]
    fn simple_commands_take_nothing() {
        assert_eq!(request(&["ctl", "status"]), AppRequestBody::Status {});
        assert_eq!(request(&["ctl", "list"]), AppRequestBody::List {});
        assert_eq!(request(&["ctl", "quit"]), AppRequestBody::Quit {});
        assert_eq!(
            request(&["ctl", "workspaces"]),
            AppRequestBody::Workspaces {}
        );
        assert!(parsed(&["ctl", "list", "all"]).is_err());
        assert!(parsed(&["ctl"]).is_err());
        assert!(parsed(&["ctl", "dance"]).is_err());
    }

    #[test]
    fn the_endpoint_and_the_timeout_come_before_the_command() {
        let Ok(Command::Ctl(ctl)) =
            parsed(&["ctl", "--endpoint", "trial", "--timeout", "2.5", "list"])
        else {
            panic!("not a request");
        };
        assert_eq!(ctl.endpoint.as_deref(), Some("trial"));
        assert_eq!(ctl.timeout, Duration::from_millis(2500));

        let Ok(Command::Ctl(ctl)) = parsed(&["--endpoint", "outer", "ctl", "list"]) else {
            panic!("not a request");
        };
        assert_eq!(ctl.endpoint.as_deref(), Some("outer"));
        assert!(parsed(&["ctl", "--timeout", "soon", "list"]).is_err());
        assert!(parsed(&["ctl", "--timeout", "-1", "list"]).is_err());
    }

    #[test]
    fn spawn_takes_a_directory_a_size_variables_and_a_program() {
        assert_eq!(
            request(&["ctl", "spawn"]),
            AppRequestBody::Spawn {
                spec: OpenSpec::default(),
                place: SpawnPlace::Tab {},
            }
        );
        assert_eq!(
            request(&["ctl", "spawn", "--split", "down"]),
            AppRequestBody::Spawn {
                spec: OpenSpec::default(),
                place: SpawnPlace::Split {
                    dir: Direction::Down
                },
            }
        );
        assert_eq!(
            request(&["ctl", "spawn", "--window"]),
            AppRequestBody::Spawn {
                spec: OpenSpec::default(),
                place: SpawnPlace::Window {},
            }
        );
        assert!(parsed(&["ctl", "spawn", "--split", "sideways"]).is_err());
        assert!(parsed(&["ctl", "spawn", "--tab", "--split", "up"]).is_err());
        let spawned = request(&[
            "ctl", "spawn", "--cwd", "/tmp", "--size", "100x30", "--env", "A=1", "--env", "B=x=y",
            "--", "bash", "-l", "--norc",
        ]);
        assert_eq!(
            spawned,
            AppRequestBody::Spawn {
                spec: OpenSpec {
                    profile: None,
                    launcher: Some(Launcher::Shell {
                        program: "bash".to_owned(),
                        args: vec!["-l".to_owned(), "--norc".to_owned()],
                        login: false,
                    }),
                    cwd: Some("/tmp".to_owned()),
                    env: vec![
                        ("A".to_owned(), "1".to_owned()),
                        ("B".to_owned(), "x=y".to_owned())
                    ],
                    size: Some(TermSize::new(100, 30).unwrap()),
                },
                place: SpawnPlace::Tab {},
            }
        );
        assert!(parsed(&["ctl", "spawn", "--size", "0x30"]).is_err());
        assert!(parsed(&["ctl", "spawn", "--env", "=1"]).is_err());
        assert!(parsed(&["ctl", "spawn", "--"]).is_err());
        assert!(parsed(&["ctl", "spawn", "bash"]).is_err());
    }

    #[test]
    fn send_joins_its_words_and_reads_escapes_when_asked() {
        assert_eq!(
            request(&["ctl", "send", "echo", "hello"]),
            AppRequestBody::Send {
                pane: None,
                session: None,
                text: "echo hello".to_owned()
            }
        );
        assert_eq!(
            request(&["ctl", "send", "--pane", "p12", "x"]),
            AppRequestBody::Send {
                pane: Some("p12".to_owned()),
                session: None,
                text: "x".to_owned()
            }
        );
        assert!(parsed(&["ctl", "send", "--pane", "12", "x"]).is_err());
        assert!(parsed(&["ctl", "send", "--pane", "p1x", "x"]).is_err());
        assert!(
            parsed(&[
                "ctl",
                "send",
                "--pane",
                "p1",
                "--session",
                "00000000000000ab",
                "x"
            ])
            .is_err()
        );
        assert_eq!(
            request(&[
                "ctl",
                "send",
                "--session",
                "00000000000000ab",
                "--enter",
                "ls"
            ]),
            AppRequestBody::Send {
                pane: None,
                session: Some(id()),
                text: "ls\r".to_owned()
            }
        );
        assert_eq!(
            request(&["ctl", "send", "--escapes", r"a\tb\x03\e[A\\"]),
            AppRequestBody::Send {
                pane: None,
                session: None,
                text: "a\tb\x03\x1b[A\\".to_owned()
            }
        );
        // Without --escapes a backslash is a backslash, and after the text begins an
        // option is text.
        assert_eq!(
            request(&["ctl", "send", r"a\tb", "--enter"]),
            AppRequestBody::Send {
                pane: None,
                session: None,
                text: r"a\tb --enter".to_owned()
            }
        );
        assert_eq!(
            request(&["ctl", "send", "--", "--enter"]),
            AppRequestBody::Send {
                pane: None,
                session: None,
                text: "--enter".to_owned()
            }
        );
        assert_eq!(
            request(&["ctl", "send", "--enter"]),
            AppRequestBody::Send {
                pane: None,
                session: None,
                text: "\r".to_owned()
            }
        );
        assert!(parsed(&["ctl", "send"]).is_err());
        assert!(parsed(&["ctl", "send", "--escapes", r"\q"]).is_err());
        assert!(parsed(&["ctl", "send", "--escapes", r"\xff"]).is_err());
        assert!(parsed(&["ctl", "send", "--session", "nope", "x"]).is_err());
    }

    #[test]
    fn wait_is_for_text_or_for_the_exit() {
        assert_eq!(
            request(&["ctl", "--timeout", "3", "wait", "--text", "$ "]),
            AppRequestBody::WaitText {
                pane: None,
                session: None,
                text: "$ ".to_owned(),
                timeout_ms: 3000
            }
        );
        assert_eq!(
            request(&["ctl", "wait", "--exit", "--session", "00000000000000ab"]),
            AppRequestBody::WaitExit {
                pane: None,
                session: Some(id()),
                timeout_ms: 10_000
            }
        );
        assert_eq!(
            request(&["ctl", "wait", "--pane", "p3", "--exit"]),
            AppRequestBody::WaitExit {
                pane: Some("p3".to_owned()),
                session: None,
                timeout_ms: 10_000
            }
        );
        assert!(parsed(&["ctl", "wait"]).is_err());
        assert!(parsed(&["ctl", "wait", "--text", "a", "--exit"]).is_err());
        assert!(parsed(&["ctl", "wait", "--text", ""]).is_err());
    }

    #[test]
    fn capture_resize_and_kill_name_their_session() {
        assert_eq!(
            request(&["ctl", "capture"]),
            AppRequestBody::Screen {
                pane: None,
                session: None
            }
        );
        assert_eq!(
            request(&["ctl", "capture", "--pane", "p2"]),
            AppRequestBody::Screen {
                pane: Some("p2".to_owned()),
                session: None
            }
        );
        let Ok(Command::Ctl(ctl)) = parsed(&["ctl", "capture", "--plain"]) else {
            panic!("not a request");
        };
        assert!(ctl.plain);
        assert_eq!(
            request(&["ctl", "resize", "120x40"]),
            AppRequestBody::Resize {
                pane: None,
                session: None,
                size: TermSize::new(120, 40).unwrap()
            }
        );
        assert_eq!(
            request(&["ctl", "resize", "--session", "00000000000000ab", "90X20"]),
            AppRequestBody::Resize {
                pane: None,
                session: Some(id()),
                size: TermSize::new(90, 20).unwrap()
            }
        );
        assert_eq!(
            request(&["ctl", "kill", "00000000000000ab"]),
            AppRequestBody::Kill { session: id() }
        );
        assert!(parsed(&["ctl", "resize"]).is_err());
        assert!(parsed(&["ctl", "resize", "wide"]).is_err());
        assert!(parsed(&["ctl", "kill"]).is_err());
        assert!(parsed(&["ctl", "kill", "zz"]).is_err());
    }

    #[test]
    fn the_layout_is_viewed_or_given_an_intent() {
        assert_eq!(
            request(&["ctl", "layout", "view"]),
            AppRequestBody::Layout {}
        );
        assert_eq!(
            request(&["ctl", "layout", "apply", r#"{"type":"close","pane":"p3"}"#]),
            AppRequestBody::LayoutApply {
                intent: serde_json::json!({"type": "close", "pane": "p3"})
            }
        );
        let Ok(Command::Ctl(ctl)) = parsed(&["ctl", "layout", "apply", "-"]) else {
            panic!("not a request");
        };
        assert!(ctl.intent_from_input);
        let Ok(Command::Ctl(ctl)) = parsed(&["ctl", "layout", "view"]) else {
            panic!("not a request");
        };
        assert!(!ctl.intent_from_input);
        assert!(parsed(&["ctl", "layout"]).is_err());
        assert!(parsed(&["ctl", "layout", "apply"]).is_err());
        assert!(parsed(&["ctl", "layout", "apply", "{not json"]).is_err());
        assert!(parsed(&["ctl", "layout", "undo"]).is_err());
        assert!(parsed(&["ctl", "layout", "view", "all"]).is_err());
    }

    #[test]
    fn a_command_is_run_by_its_id_with_arguments_as_json() {
        assert_eq!(request(&["ctl", "commands"]), AppRequestBody::Commands {});
        assert_eq!(
            request(&["ctl", "command", "lume.tab.new"]),
            AppRequestBody::Command {
                id: "lume.tab.new".to_owned(),
                args: serde_json::Value::Null,
                window: None,
            }
        );
        assert_eq!(
            request(&["ctl", "command", "lume.tab.activate", r#"{"index":2}"#]),
            AppRequestBody::Command {
                id: "lume.tab.activate".to_owned(),
                args: serde_json::json!({"index": 2}),
                window: None,
            }
        );
        assert!(parsed(&["ctl", "command"]).is_err());
        assert!(parsed(&["ctl", "command", "lume.tab.activate", "2"]).is_err());
        assert!(parsed(&["ctl", "command", "lume.tab.new", "{}", "more"]).is_err());
    }

    fn offline(args: &[&str]) -> Offline {
        match parsed(args) {
            Ok(Command::Offline(offline)) => offline,
            other => panic!("not a command on the configuration: {other:?}"),
        }
    }

    #[test]
    fn config_prints_the_schema_or_the_path_or_checks_a_file() {
        assert_eq!(offline(&["config", "schema"]), Offline::Schema);
        assert_eq!(offline(&["config", "path"]), Offline::Path);
        assert_eq!(offline(&["config", "check"]), Offline::Check { file: None });
        assert_eq!(
            offline(&["config", "check", "my.toml"]),
            Offline::Check {
                file: Some("my.toml".to_owned())
            }
        );
        assert!(parsed(&["config"]).is_err());
        assert!(parsed(&["config", "edit"]).is_err());
        assert!(parsed(&["config", "schema", "now"]).is_err());
        assert!(parsed(&["config", "check", "a.toml", "b.toml"]).is_err());
    }

    #[test]
    fn keymap_takes_a_platform_a_workspace_and_a_format() {
        assert_eq!(
            offline(&["keymap"]),
            Offline::Keymap {
                platform: None,
                workspace: None,
                json: false
            }
        );
        assert_eq!(
            offline(&[
                "keymap",
                "--json",
                "--platform",
                "macos",
                "--workspace",
                "work"
            ]),
            Offline::Keymap {
                platform: Some(Platform::Macos),
                workspace: Some("work".to_owned()),
                json: true
            }
        );
        assert!(parsed(&["keymap", "--platform"]).is_err());
        assert!(parsed(&["keymap", "--platform", "beos"]).is_err());
        assert!(parsed(&["keymap", "all"]).is_err());
    }

    #[test]
    fn theme_import_takes_a_file_and_perhaps_a_name() {
        assert_eq!(
            offline(&["theme", "import", "tokyo-night"]),
            Offline::ThemeImport {
                file: "tokyo-night".to_owned(),
                name: None
            }
        );
        assert_eq!(
            offline(&["theme", "import", "--name", "Tokyo Night", "scheme.conf"]),
            Offline::ThemeImport {
                file: "scheme.conf".to_owned(),
                name: Some("Tokyo Night".to_owned())
            }
        );
        assert!(parsed(&["theme"]).is_err());
        assert!(parsed(&["theme", "export"]).is_err());
        assert!(parsed(&["theme", "import"]).is_err());
        assert!(parsed(&["theme", "import", "a", "b"]).is_err());
        assert!(parsed(&["theme", "import", "a", "--name"]).is_err());
    }

    #[test]
    fn shell_integration_takes_the_shell_whose_script_to_print() {
        let script = |shell| Offline::ShellIntegration { shell };
        assert_eq!(
            offline(&["shell-integration", "bash"]),
            script(IntegrationShell::Bash)
        );
        assert_eq!(
            offline(&["shell-integration", "zsh"]),
            script(IntegrationShell::Zsh)
        );
        assert_eq!(
            offline(&["shell-integration", "fish"]),
            script(IntegrationShell::Fish)
        );
        // PowerShell goes by two names.
        for name in ["powershell", "pwsh"] {
            assert_eq!(
                offline(&["shell-integration", name]),
                script(IntegrationShell::PowerShell)
            );
        }
        assert!(parsed(&["shell-integration"]).is_err());
        assert!(parsed(&["shell-integration", "cmd"]).is_err());
        assert!(parsed(&["shell-integration", "bash", "zsh"]).is_err());
    }

    #[test]
    fn events_follow_every_session_or_the_one_named_for_as_long_as_was_said() {
        let followed = |args: &[&str]| match parsed(args) {
            Ok(Command::Ctl(ctl)) => (ctl.request, ctl.follow_for),
            other => panic!("not a request: {other:?}"),
        };
        assert_eq!(
            followed(&["ctl", "events"]),
            (
                AppRequestBody::Events {
                    pane: None,
                    session: None
                },
                None
            )
        );
        assert_eq!(
            followed(&["ctl", "--timeout", "5", "events", "--pane", "p3"]),
            (
                AppRequestBody::Events {
                    pane: Some("p3".to_owned()),
                    session: None
                },
                Some(Duration::from_secs(5))
            )
        );
        assert_eq!(
            followed(&["ctl", "events", "--session", "00000000000000ab"]).0,
            AppRequestBody::Events {
                pane: None,
                session: Some(id())
            }
        );
        assert!(parsed(&["ctl", "events", "all"]).is_err());
        assert!(parsed(&["ctl", "events", "--pane"]).is_err());
    }
}
