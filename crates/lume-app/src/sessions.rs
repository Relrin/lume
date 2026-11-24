//! What the sessions have reported: for each, the state its reports add up to.
//!
//! A session's reports go to the page that shows it, which draws them. The application
//! itself needs a few of them too: where a pane's shell is, so that a terminal opened
//! beside it starts there and the workspace's file remembers it; and all of them for
//! whoever follows the sessions from outside. So the reports are noted here as they
//! pass on their way to the page.
//!
//! Nothing here is more than what a program printed. The one use the application makes
//! of it, a directory to start in, goes through the check in `lume_session::cwd`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use lume_proto::app::SessionNews;
use lume_session::{
    BackendId, CommandState, ExitStatus, Foreground, Launcher, SessionEvent, SessionId,
    SessionState, is_shell_program, program_stem,
};
use tokio::sync::broadcast;

use crate::carrier::SessionKey;
use crate::tuning::{SESSION_NEWS_BACKLOG, SHELL_ALIASES_MAX};

/// Something a session reported, or its program's end.
#[derive(Clone, Debug)]
pub struct News {
    pub key: SessionKey,
    pub what: SessionNews,
}

#[derive(Default)]
struct Known {
    /// What the session runs, where the application started it and so knows.
    launcher: Option<Launcher>,
    /// The profile it was started from, where it was started from one.
    profile: Option<String>,
    /// The process the session started, once it is known.
    pid: Option<u32>,
    /// Processes under the shell's own name that had the terminal before the shell
    /// ran any command line. A shell may start the real one as a child of its own (Git
    /// for Windows' `bin\bash.exe` does), and that child is then the shell at its
    /// prompt. A shell of the same name started later from a command line is not.
    shell_pids: Vec<u32>,
    state: SessionState,
}

impl Known {
    /// Whether the program that has the terminal is the session's shell itself.
    fn is_shell(&self, foreground: &Foreground) -> bool {
        self.pid == Some(foreground.pid) || self.shell_pids.contains(&foreground.pid)
    }

    /// Notes the program that has the terminal, if it is the shell at its prompt.
    fn note_foreground(&mut self) {
        let (Some(foreground), Some(Launcher::Shell { program, .. })) =
            (&self.state.foreground, &self.launcher)
        else {
            return;
        };
        // At its prompt the shell itself has the terminal: before any command, and once
        // one has finished, which the shell says from its prompt. A shell first seen only
        // after the first command has ended (typed before the prompt was looked at) is
        // known then.
        let at_prompt = matches!(
            self.state.command,
            None | Some(CommandState::Finished { .. })
        );
        let shell_named = program_stem(&foreground.name) == program_stem(program);
        if at_prompt
            && shell_named
            && !self.shell_pids.contains(&foreground.pid)
            && self.shell_pids.len() < SHELL_ALIASES_MAX
        {
            self.shell_pids.push(foreground.pid);
        }
    }
}

/// Whether something runs in a session that closing its pane would end.
///
/// - A session whose program has ended runs nothing.
/// - Of one the application did not start, or one on another machine, nothing can be
///   told: it is taken to run something.
/// - A command line the shell said it is running is something.
/// - In a Linux distribution the program here is always the distribution's own, so only
///   the shell's word counts: it runs nothing once it said its command finished, or
///   said where it is before it ran any, as a shell at its first prompt does.
/// - A program that is not a shell is the something.
/// - A shell runs something while another program has the terminal, whatever it said
///   last: a program it runs can print a prompt's marks too.
/// - With nothing known of the terminal, only the shell's word counts.
pub fn runs_something(
    ended: bool,
    launcher: Option<&Launcher>,
    state: &SessionState,
    is_shell: impl Fn(&Foreground) -> bool,
) -> bool {
    if ended {
        return false;
    }
    let finished = matches!(state.command, Some(CommandState::Finished { .. }));
    match launcher {
        None | Some(Launcher::Ssh { .. }) => true,
        Some(_) if state.command_running() => true,
        Some(Launcher::Wsl { .. }) => {
            let at_first_prompt = state.command.is_none() && state.cwd.is_some();
            !(finished || at_first_prompt)
        }
        Some(Launcher::Shell { program, .. }) if !is_shell_program(program) => true,
        Some(Launcher::Shell { .. }) => match &state.foreground {
            Some(foreground) => !is_shell(foreground),
            None => !finished,
        },
        // A way of starting a program this does not know.
        Some(_) => true,
    }
}

/// What the application started a session to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launched {
    pub launcher: Launcher,
    pub profile: Option<String>,
}

pub struct SessionStates {
    sessions: Mutex<HashMap<SessionKey, Known>>,
    news: broadcast::Sender<News>,
}

impl Default for SessionStates {
    fn default() -> Self {
        let (news, _) = broadcast::channel(SESSION_NEWS_BACKLOG);
        Self {
            sessions: Mutex::new(HashMap::new()),
            news,
        }
    }
}

impl SessionStates {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn sessions(&self) -> MutexGuard<'_, HashMap<SessionKey, Known>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Notes what a session that was just started runs, and the profile it was started
    /// from.
    pub fn started(&self, key: SessionKey, launcher: Launcher, profile: Option<String>) {
        let mut sessions = self.sessions();
        let known = sessions.entry(key).or_default();
        known.launcher = Some(launcher);
        known.profile = profile;
    }

    /// Notes that a session is being shown: what it reports from now on is kept.
    pub fn shown(&self, key: &SessionKey) {
        self.sessions().entry(key.clone()).or_default();
    }

    /// What the application started a session to run, if it started it.
    pub fn launched(&self, key: &SessionKey) -> Option<Launched> {
        let sessions = self.sessions();
        let known = sessions.get(key)?;
        Some(Launched {
            launcher: known.launcher.clone()?,
            profile: known.profile.clone(),
        })
    }

    /// Notes the process a session started.
    pub fn set_pid(&self, key: &SessionKey, pid: u32) {
        if let Some(known) = self.sessions().get_mut(key) {
            known.pid = Some(pid);
        }
    }

    /// Whether something runs in a session that closing its pane would end, going by
    /// what the session reported. `ended` says its program has ended.
    pub fn busy(&self, key: &SessionKey, ended: bool) -> bool {
        let sessions = self.sessions();
        match sessions.get(key) {
            Some(known) => runs_something(ended, known.launcher.as_ref(), &known.state, |fg| {
                known.is_shell(fg)
            }),
            None => !ended,
        }
    }

    /// Takes in a report on its way to the page. One of a session that was forgotten
    /// meanwhile is not kept: nothing would forget it again.
    pub fn on_event(&self, key: &SessionKey, event: &SessionEvent) {
        if let Some(known) = self.sessions().get_mut(key) {
            known.state.apply(event);
            known.note_foreground();
        }
        // Nobody following is not an error.
        let _ = self.news.send(News {
            key: key.clone(),
            what: SessionNews::Event(event.clone()),
        });
    }

    /// Tells whoever follows the sessions that a session's program has ended. What the
    /// session reported is kept: its pane may stay, and say how it ended.
    pub fn on_exit(&self, key: &SessionKey, status: ExitStatus) {
        let _ = self.news.send(News {
            key: key.clone(),
            what: SessionNews::Exit(status),
        });
    }

    /// Forgets a session that is gone.
    pub fn forget(&self, key: &SessionKey) {
        self.sessions().remove(key);
    }

    /// Forgets every session of a backend but these.
    pub fn keep_of(&self, backend: &BackendId, kept: &HashSet<SessionId>) {
        self.sessions()
            .retain(|key, _| key.backend != *backend || kept.contains(&key.session));
    }

    /// What a session's reports add up to so far.
    pub fn state(&self, key: &SessionKey) -> Option<SessionState> {
        self.sessions().get(key).map(|known| known.state.clone())
    }

    /// What a session runs, and the directory it last reported, where both are known.
    pub fn reported_dir(&self, key: &SessionKey) -> Option<(Launcher, String)> {
        let sessions = self.sessions();
        let known = sessions.get(key)?;
        Some((known.launcher.clone()?, known.state.cwd.clone()?))
    }

    /// Every session that has reported something, with its state, and the reports
    /// from now on. The two are taken together, so no report falls between them.
    pub fn follow(&self) -> (Vec<(SessionKey, SessionState)>, broadcast::Receiver<News>) {
        let sessions = self.sessions();
        let news = self.news.subscribe();
        let states = sessions
            .iter()
            .filter(|(_, known)| !known.state.is_empty())
            .map(|(key, known)| (key.clone(), known.state.clone()))
            .collect();
        (states, news)
    }

    /// How many sessions are known.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.sessions().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(raw: u64) -> SessionKey {
        SessionKey {
            backend: BackendId("local".to_owned()),
            session: SessionId::new(raw).unwrap(),
        }
    }

    /// Sessions that are shown: these, of the one backend.
    fn states(raws: &[u64]) -> Arc<SessionStates> {
        let states = SessionStates::new();
        for raw in raws {
            states.shown(&key(*raw));
        }
        states
    }

    fn cwd(url: &str) -> SessionEvent {
        SessionEvent::Cwd {
            url: url.to_owned(),
        }
    }

    fn shell() -> Launcher {
        Launcher::Shell {
            program: "sh".to_owned(),
            args: Vec::new(),
            login: false,
        }
    }

    #[test]
    fn reports_add_up_to_a_state_per_session() {
        let states = states(&[1, 2]);
        assert_eq!(states.state(&key(3)), None);
        states.on_event(&key(1), &cwd("file://box/a"));
        states.on_event(&key(1), &cwd("file://box/b"));
        states.on_event(&key(2), &SessionEvent::Bell);
        assert_eq!(
            states.state(&key(1)).unwrap().cwd.as_deref(),
            Some("file://box/b")
        );
        assert!(states.state(&key(2)).unwrap().is_empty());

        states.forget(&key(1));
        assert_eq!(states.state(&key(1)), None);
        assert_eq!(states.len(), 1);
        // A report that was on its way brings nothing back.
        states.on_event(&key(1), &cwd("file://box/c"));
        assert_eq!(states.state(&key(1)), None);
    }

    #[test]
    fn the_sessions_of_one_backend_are_forgotten_but_for_those_that_are_kept() {
        let states = SessionStates::new();
        let server = |raw: u64| SessionKey {
            backend: BackendId("server".to_owned()),
            session: SessionId::new(raw).unwrap(),
        };
        for known in [key(1), key(2), server(1), server(2)] {
            states.shown(&known);
            states.on_event(&known, &cwd("file://box/a"));
        }
        let kept = HashSet::from([SessionId::new(2).unwrap()]);
        states.keep_of(&server(1).backend, &kept);
        let known = |key: SessionKey| states.state(&key).is_some();
        assert!(known(key(1)) && known(key(2)) && known(server(2)));
        assert!(!known(server(1)));
    }

    #[test]
    fn a_directory_is_only_known_with_what_the_session_runs() {
        let states = states(&[1]);
        states.on_event(&key(1), &cwd("file://box/a"));
        // A session the application did not start: nothing says what it runs.
        assert_eq!(states.reported_dir(&key(1)), None);
        assert_eq!(states.launched(&key(1)), None);
        states.started(key(1), shell(), None);
        assert_eq!(
            states.reported_dir(&key(1)),
            Some((shell(), "file://box/a".to_owned()))
        );
        states.started(key(2), shell(), Some("box".to_owned()));
        assert_eq!(states.reported_dir(&key(2)), None);
        assert_eq!(
            states.launched(&key(2)),
            Some(Launched {
                launcher: shell(),
                profile: Some("box".to_owned()),
            })
        );
    }

    fn program(program: &str) -> Launcher {
        Launcher::Shell {
            program: program.to_owned(),
            args: Vec::new(),
            login: false,
        }
    }

    fn distro() -> Launcher {
        Launcher::Wsl {
            distro: "Ubuntu".to_owned(),
            user: None,
        }
    }

    fn remote() -> Launcher {
        Launcher::Ssh {
            host: "box".to_owned(),
            user: None,
            port: None,
            extra_args: Vec::new(),
            client: None,
        }
    }

    fn foreground(pid: u32, name: &str) -> Foreground {
        Foreground {
            pid,
            name: name.to_owned(),
        }
    }

    fn running(command: Option<&str>) -> Option<CommandState> {
        Some(CommandState::Running {
            command: command.map(str::to_owned),
        })
    }

    fn finished(exit: i32) -> Option<CommandState> {
        Some(CommandState::Finished { exit: Some(exit) })
    }

    #[test]
    fn what_runs_in_a_session_goes_by_what_it_runs_and_what_it_said() {
        const SHELL_PID: u32 = 10;
        let is_shell = |found: &Foreground| found.pid == SHELL_PID;
        let state = |command: Option<CommandState>, found: Option<Foreground>| SessionState {
            command,
            foreground: found,
            ..SessionState::default()
        };
        let idle = state(None, None);
        let cases: Vec<(&str, bool, Option<Launcher>, SessionState, bool)> = vec![
            // 1. A program that has ended runs nothing, whatever else is known.
            (
                "ended",
                true,
                Some(remote()),
                state(running(None), None),
                false,
            ),
            ("ended shell", true, Some(shell()), idle.clone(), false),
            // 2. Nothing can be told of a session the application did not start, nor of
            // one on another machine.
            ("not started here", false, None, idle.clone(), true),
            ("ssh", false, Some(remote()), state(finished(0), None), true),
            // 3. The shell's own word that a command runs.
            (
                "running",
                false,
                Some(shell()),
                state(running(Some("make")), Some(foreground(SHELL_PID, "sh"))),
                true,
            ),
            // 4. In a distribution only the shell's word counts.
            ("wsl at first", false, Some(distro()), idle.clone(), true),
            (
                "wsl at its first prompt",
                false,
                Some(distro()),
                SessionState {
                    cwd: Some("file://box/home/me".to_owned()),
                    foreground: Some(foreground(99, "wsl")),
                    ..SessionState::default()
                },
                false,
            ),
            (
                "wsl done",
                false,
                Some(distro()),
                state(finished(0), Some(foreground(99, "wsl"))),
                false,
            ),
            // 5. A pane that runs a program of its own runs it.
            (
                "a program",
                false,
                Some(program("vim")),
                state(finished(0), Some(foreground(SHELL_PID, "vim"))),
                true,
            ),
            // 6. A shell runs something while another program has the terminal, even
            // after it said a command finished.
            (
                "idle shell",
                false,
                Some(shell()),
                state(None, Some(foreground(SHELL_PID, "sh"))),
                false,
            ),
            (
                "another program",
                false,
                Some(shell()),
                state(finished(0), Some(foreground(11, "vim"))),
                true,
            ),
            // 7. With nothing known of the terminal, only the shell's word.
            ("shell unknown", false, Some(shell()), idle.clone(), true),
            (
                "shell said done",
                false,
                Some(shell()),
                state(finished(2), None),
                false,
            ),
        ];
        for (name, ended, launcher, state, busy) in cases {
            assert_eq!(
                runs_something(ended, launcher.as_ref(), &state, is_shell),
                busy,
                "{name}"
            );
        }
    }

    #[test]
    fn a_shell_that_runs_the_real_one_as_its_child_is_still_at_its_prompt() {
        let states = SessionStates::new();
        let git_bash = program(r"C:\Program Files\Git\bin\bash.exe");
        let seen = |raw: u64, pid: u32, name: &str| {
            states.on_event(&key(raw), &SessionEvent::Foreground(foreground(pid, name)));
        };
        let command = |raw: u64| {
            states.on_event(&key(raw), &SessionEvent::CommandStarted { command: None });
            states.on_event(&key(raw), &SessionEvent::CommandFinished { exit: Some(0) });
        };
        states.started(key(1), git_bash.clone(), None);
        // The shell's own process is seen first, before it has started the real one,
        // and before what it is called is known.
        seen(1, 10, "bash");
        states.set_pid(&key(1), 10);
        assert!(!states.busy(&key(1), false));
        // The real one, its child, is the shell at its prompt too.
        seen(1, 11, "bash");
        assert!(!states.busy(&key(1), false));
        command(1);
        seen(1, 12, "vim");
        assert!(states.busy(&key(1), false));
        seen(1, 11, "bash");
        assert!(!states.busy(&key(1), false));
        // Another bash started from a command line is a program the shell runs: the
        // command that started it has not finished.
        states.on_event(&key(1), &SessionEvent::CommandStarted { command: None });
        seen(1, 13, "bash");
        assert!(states.busy(&key(1), false));
        assert!(!states.busy(&key(1), true));

        // A first program of another name is not taken for the shell.
        states.started(key(2), git_bash, None);
        states.set_pid(&key(2), 20);
        seen(2, 21, "vim");
        assert!(states.busy(&key(2), false));
        seen(2, 20, "bash");
        assert!(!states.busy(&key(2), false));

        // A shell first seen only once the first command has finished, because the
        // command was typed before the prompt was looked at, is known then.
        states.started(key(4), program(r"C:\Program Files\Git\bin\bash.exe"), None);
        states.set_pid(&key(4), 40);
        command(4);
        seen(4, 41, "bash");
        assert!(!states.busy(&key(4), false));

        // What a whole state says counts as a report of the program too.
        states.started(key(3), shell(), None);
        states.on_event(
            &key(3),
            &SessionEvent::State {
                state: SessionState {
                    foreground: Some(foreground(31, "sh")),
                    ..SessionState::default()
                },
            },
        );
        assert!(!states.busy(&key(3), false));
        // A session nothing is known of runs something until it has ended.
        assert!(states.busy(&key(9), false));
        assert!(!states.busy(&key(9), true));
    }

    #[tokio::test]
    async fn a_follower_gets_the_states_so_far_and_then_every_report() {
        let states = states(&[1, 2]);
        states.on_event(&key(1), &cwd("file://box/a"));
        states.on_event(&key(2), &SessionEvent::Bell);
        let (so_far, mut news) = states.follow();
        // A session that has only rung its bell has no state to tell.
        assert_eq!(so_far.len(), 1);
        assert_eq!(so_far[0].0, key(1));

        states.on_event(&key(2), &cwd("file://box/c"));
        states.on_exit(&key(1), ExitStatus::Code(3));
        let first = news.recv().await.unwrap();
        assert_eq!(first.key, key(2));
        assert_eq!(first.what, SessionNews::Event(cwd("file://box/c")));
        let second = news.recv().await.unwrap();
        assert_eq!(second.key, key(1));
        assert_eq!(second.what, SessionNews::Exit(ExitStatus::Code(3)));
        // The end of the program does not take what it reported with it.
        assert!(states.state(&key(1)).is_some());
    }
}
