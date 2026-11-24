//! Asking for the user's attention to a window they are not using.
//!
//! A program that rings the bell in a pane of such a window has the window ask for
//! attention: its taskbar button flashes, or its dock icon bounces. A command that ran a
//! while and ends there may be told in a notification on the desktop, if the user asked
//! for that.
//!
//! What a session reports is only what a program printed. So a notification says which
//! pane it is about, how the command ended and how long it ran, and never the command
//! line, which any program, on this machine or another, could have written.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lume_config::Config;
use lume_layout::WindowId;
use lume_proto::app::SessionNews;
use lume_session::{Launcher, SessionEvent, program_stem};
use tokio::sync::broadcast::error::RecvError;

use crate::carrier::SessionKey;
use crate::control::Shell;
use crate::services::Services;
use crate::sessions::News;
use crate::tuning::{BELL_ATTENTION_INTERVAL, NOTIFY_INTERVAL};

/// What the settings say about asking for attention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Whether a bell has its window ask for attention.
    pub bell: bool,
    /// Whether a long command that ends is notified.
    pub notify: bool,
    /// How long a command runs before its end is worth a notification.
    pub long_command: Duration,
}

impl Settings {
    pub fn of(config: &Config) -> Self {
        Self {
            bell: config.window.bell_attention,
            notify: config.terminal.notify_long_command,
            long_command: Duration::from_secs(u64::from(config.terminal.long_command_seconds)),
        }
    }
}

/// What to do about something a session reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Have a window ask for attention.
    Attention(WindowId),
    /// Show a notification on the desktop.
    Notify { title: String, body: String },
}

/// What the decision needs to know of the application.
pub trait Surroundings {
    /// The window of the pane that shows a session, if a pane does.
    fn window_of(&self, key: &SessionKey) -> Option<WindowId>;
    /// Whether a window is the one the user is working in.
    fn window_focused(&self, window: WindowId) -> bool;
    /// What a person calls the pane that shows a session.
    fn name_of(&self, key: &SessionKey) -> String;
    /// Whether the pane that shows a session is in a tab the user muted, or in a group
    /// they did. Nothing asks for attention on behalf of such a pane.
    fn muted(&self, key: &SessionKey) -> bool;
}

/// What is kept between reports to decide what to do about the next.
#[derive(Default)]
pub struct Attention {
    /// When each session's running command began, as far as it was seen begin.
    started: HashMap<SessionKey, Instant>,
    /// When each window last asked for attention.
    asked: HashMap<WindowId, Instant>,
    /// When the last notification was shown.
    notified: Option<Instant>,
}

/// A duration as a person reads it: `45 s`, `3 min 20 s`, `1 h 5 min`.
fn spoken(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    match (hours, minutes, seconds) {
        (0, 0, seconds) => format!("{seconds} s"),
        (0, minutes, 0) => format!("{minutes} min"),
        (0, minutes, seconds) => format!("{minutes} min {seconds} s"),
        (hours, 0, _) => format!("{hours} h"),
        (hours, minutes, _) => format!("{hours} h {minutes} min"),
    }
}

/// Whether something last done at `last` may be done again at `now`.
fn rested(last: Option<&Instant>, now: Instant, interval: Duration) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(*last) >= interval)
}

impl Attention {
    /// Decides what to do about one report.
    pub fn take(
        &mut self,
        news: &News,
        now: Instant,
        settings: &Settings,
        around: &dyn Surroundings,
    ) -> Option<Act> {
        let key = &news.key;
        let event = match &news.what {
            SessionNews::Exit(_) => {
                self.started.remove(key);
                return None;
            }
            SessionNews::Event(event) => event,
        };
        match event {
            SessionEvent::Bell => {
                if !settings.bell || around.muted(key) {
                    return None;
                }
                let window = around.window_of(key)?;
                if around.window_focused(window)
                    || !rested(self.asked.get(&window), now, BELL_ATTENTION_INTERVAL)
                {
                    return None;
                }
                self.asked.insert(window, now);
                Some(Act::Attention(window))
            }
            SessionEvent::CommandStarted { .. } => {
                // A session that went without its end being seen is not kept for ever.
                self.started
                    .retain(|other, _| around.window_of(other).is_some());
                self.started.insert(key.clone(), now);
                None
            }
            SessionEvent::CommandFinished { exit } => {
                let began = self.started.remove(key)?;
                let ran = now.saturating_duration_since(began);
                if !settings.notify || ran < settings.long_command || around.muted(key) {
                    return None;
                }
                let window = around.window_of(key)?;
                if around.window_focused(window)
                    || !rested(self.notified.as_ref(), now, NOTIFY_INTERVAL)
                {
                    return None;
                }
                self.notified = Some(now);
                let title = match exit {
                    Some(code) if *code != 0 => format!("A command failed with exit code {code}"),
                    _ => "A command finished".to_owned(),
                };
                let body = format!("{}: it ran for {}", around.name_of(key), spoken(ran));
                Some(Act::Notify { title, body })
            }
            // How things stand is not news. A command it says is not running has ended
            // unseen; one it says is running began at a time that is not known.
            SessionEvent::State { state } => {
                if !state.command_running() {
                    self.started.remove(key);
                }
                None
            }
            _ => None,
        }
    }

    /// Forgets every command seen begin: reports were missed, and an end may be among
    /// them.
    pub fn forget(&mut self) {
        self.started.clear();
    }
}

/// The application, as the decision sees it.
struct Live {
    services: Arc<Services>,
}

impl Surroundings for Live {
    fn window_of(&self, key: &SessionKey) -> Option<WindowId> {
        let pane = self.services.panes.pane_of(key)?;
        self.services.panes.workspace().window_of(pane)
    }

    fn window_focused(&self, window: WindowId) -> bool {
        self.services.shell.window_focused(window)
    }

    fn muted(&self, key: &SessionKey) -> bool {
        let panes = &self.services.panes;
        panes
            .pane_of(key)
            .is_some_and(|pane| panes.workspace().pane_muted(pane))
    }

    fn name_of(&self, key: &SessionKey) -> String {
        let panes = &self.services.panes;
        let named = panes
            .pane_of(key)
            .and_then(|pane| panes.workspace().pane(pane)?.title.clone())
            .filter(|title| !title.trim().is_empty());
        if let Some(named) = named {
            return named;
        }
        let Some(launched) = self.services.state.reports().launched(key) else {
            return "A terminal".to_owned();
        };
        if let Some(profile) = launched.profile.and_then(|id| panes.profiles.find(&id)) {
            return profile.name;
        }
        match launched.launcher {
            Launcher::Shell { program, .. } => program_stem(&program),
            Launcher::Wsl { distro, .. } => distro,
            Launcher::Ssh { host, .. } => host,
            _ => "A terminal".to_owned(),
        }
    }
}

/// Follows what the sessions report and asks for attention as the settings say, for as
/// long as the application runs.
pub fn watch(services: &Arc<Services>, shell: Arc<dyn Shell>) {
    let live = Live {
        services: Arc::clone(services),
    };
    services.state.runtime().spawn(async move {
        let reports = Arc::clone(live.services.state.reports());
        let (_, mut news) = reports.follow();
        let mut attention = Attention::default();
        loop {
            match news.recv().await {
                Ok(news) => {
                    let settings = Settings::of(&live.services.config.current().config);
                    let act = attention.take(&news, Instant::now(), &settings, &live);
                    match act {
                        Some(Act::Attention(window)) => shell.request_attention(window),
                        Some(Act::Notify { title, body }) => shell.notify(&title, &body),
                        None => {}
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    tracing::debug!(missed, "reports were missed while asking for attention");
                    attention.forget();
                    (_, news) = reports.follow();
                }
                Err(RecvError::Closed) => return,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;

    use lume_session::{BackendId, CommandState, ExitStatus, SessionId, SessionState};

    use super::*;

    fn key(raw: u64) -> SessionKey {
        SessionKey {
            backend: BackendId("local".to_owned()),
            session: SessionId::new(raw).unwrap(),
        }
    }

    fn window(raw: u64) -> WindowId {
        WindowId::new(u32::try_from(raw).unwrap())
    }

    /// Two sessions in two windows, and a third in no window at all.
    #[derive(Default)]
    struct Around {
        focused: RefCell<HashSet<WindowId>>,
        /// The sessions whose panes are in a muted tab.
        muted: RefCell<HashSet<u64>>,
    }

    impl Around {
        fn focus(&self, raw: u64) {
            self.focused.borrow_mut().insert(window(raw));
        }

        fn mute(&self, raw: u64, on: bool) {
            let mut muted = self.muted.borrow_mut();
            if on {
                muted.insert(raw);
            } else {
                muted.remove(&raw);
            }
        }
    }

    impl Surroundings for Around {
        fn window_of(&self, key: &SessionKey) -> Option<WindowId> {
            match key.session.get() {
                3 => None,
                raw => Some(window(raw)),
            }
        }

        fn window_focused(&self, window: WindowId) -> bool {
            self.focused.borrow().contains(&window)
        }

        fn name_of(&self, _key: &SessionKey) -> String {
            "Ubuntu".to_owned()
        }

        fn muted(&self, key: &SessionKey) -> bool {
            self.muted.borrow().contains(&key.session.get())
        }
    }

    fn event(raw: u64, event: SessionEvent) -> News {
        News {
            key: key(raw),
            what: SessionNews::Event(event),
        }
    }

    fn started(raw: u64) -> News {
        event(
            raw,
            SessionEvent::CommandStarted {
                command: Some("make".to_owned()),
            },
        )
    }

    fn finished(raw: u64, exit: Option<i32>) -> News {
        event(raw, SessionEvent::CommandFinished { exit })
    }

    const ON: Settings = Settings {
        bell: true,
        notify: true,
        long_command: Duration::from_secs(10),
    };

    #[test]
    fn a_bell_has_its_window_ask_unless_it_is_in_use_or_has_just_asked() {
        let around = Around::default();
        let mut attention = Attention::default();
        let now = Instant::now();
        let bell = |raw| event(raw, SessionEvent::Bell);
        assert_eq!(
            attention.take(&bell(1), now, &ON, &around),
            Some(Act::Attention(window(1)))
        );
        // A run of bells is one request, for a while.
        let soon = now + BELL_ATTENTION_INTERVAL / 2;
        assert_eq!(attention.take(&bell(1), soon, &ON, &around), None);
        // Another window asks for itself.
        assert_eq!(
            attention.take(&bell(2), soon, &ON, &around),
            Some(Act::Attention(window(2)))
        );
        let later = now + BELL_ATTENTION_INTERVAL;
        assert_eq!(
            attention.take(&bell(1), later, &ON, &around),
            Some(Act::Attention(window(1)))
        );
        // Not in the window the user is working in, nor for a session in no window,
        // nor when the settings say not to.
        around.focus(2);
        let much_later = later + BELL_ATTENTION_INTERVAL * 2;
        assert_eq!(attention.take(&bell(2), much_later, &ON, &around), None);
        assert_eq!(attention.take(&bell(3), much_later, &ON, &around), None);
        let off = Settings { bell: false, ..ON };
        assert_eq!(attention.take(&bell(1), much_later, &off, &around), None);
    }

    #[test]
    fn a_long_command_that_ends_out_of_sight_is_notified_without_its_command_line() {
        let around = Around::default();
        let mut attention = Attention::default();
        let now = Instant::now();
        assert_eq!(attention.take(&started(1), now, &ON, &around), None);
        let act = attention.take(
            &finished(1, Some(2)),
            now + Duration::from_secs(75),
            &ON,
            &around,
        );
        let Some(Act::Notify { title, body }) = act else {
            panic!("no notification: {act:?}");
        };
        assert_eq!(title, "A command failed with exit code 2");
        assert_eq!(body, "Ubuntu: it ran for 1 min 15 s");
        assert!(!title.contains("make") && !body.contains("make"));

        // A command that succeeded, after the notifications have rested.
        let later = now + Duration::from_secs(600);
        attention.take(&started(2), later, &ON, &around);
        let act = attention.take(
            &finished(2, Some(0)),
            later + Duration::from_secs(3600),
            &ON,
            &around,
        );
        assert_eq!(
            act,
            Some(Act::Notify {
                title: "A command finished".to_owned(),
                body: "Ubuntu: it ran for 1 h".to_owned(),
            })
        );
    }

    #[test]
    fn a_command_is_not_notified_when_short_seen_in_use_or_unwanted() {
        let around = Around::default();
        let now = Instant::now();
        let long = Duration::from_secs(60);
        let run = |attention: &mut Attention, raw: u64, settings: &Settings| {
            attention.take(&started(raw), now, settings, &around);
            attention.take(&finished(raw, Some(0)), now + long, settings, &around)
        };
        // The settings say not to, by default.
        let off = Settings {
            notify: false,
            ..ON
        };
        assert_eq!(run(&mut Attention::default(), 1, &off), None);
        // A command shorter than the settings say.
        let longer = Settings {
            long_command: long * 2,
            ..ON
        };
        assert_eq!(run(&mut Attention::default(), 1, &longer), None);
        // In no window, or in the window the user is working in.
        assert_eq!(run(&mut Attention::default(), 3, &ON), None);
        around.focus(1);
        assert_eq!(run(&mut Attention::default(), 1, &ON), None);
        // An end whose beginning was not seen.
        let mut attention = Attention::default();
        assert_eq!(
            attention.take(&finished(2, Some(0)), now, &ON, &around),
            None
        );
        // Two in a row: the second waits for the first to rest.
        assert!(run(&mut attention, 2, &ON).is_some());
        assert_eq!(run(&mut attention, 2, &ON), None);
    }

    #[test]
    fn a_muted_tab_asks_for_nothing_until_it_is_let_ask_again() {
        let around = Around::default();
        let mut attention = Attention::default();
        let now = Instant::now();
        let long = Duration::from_secs(60);
        around.mute(1, true);

        // Neither a bell nor a long command's end, and the session beside it still asks.
        let bell = |raw| event(raw, SessionEvent::Bell);
        assert_eq!(attention.take(&bell(1), now, &ON, &around), None);
        assert_eq!(
            attention.take(&bell(2), now, &ON, &around),
            Some(Act::Attention(window(2)))
        );
        attention.take(&started(1), now, &ON, &around);
        assert_eq!(
            attention.take(&finished(1, Some(1)), now + long, &ON, &around),
            None
        );

        // The muted bell did not use up the window's turn, and the muted command's end
        // is not kept to be told later.
        around.mute(1, false);
        assert_eq!(
            attention.take(&bell(1), now, &ON, &around),
            Some(Act::Attention(window(1)))
        );
        assert_eq!(
            attention.take(&finished(1, Some(1)), now + long, &ON, &around),
            None
        );
        attention.take(&started(1), now + long, &ON, &around);
        assert!(
            attention
                .take(&finished(1, Some(0)), now + long * 2, &ON, &around)
                .is_some()
        );
    }

    #[test]
    fn how_things_stand_is_not_news_and_missed_reports_drop_what_was_seen_begin() {
        let around = Around::default();
        let now = Instant::now();
        let end = now + Duration::from_secs(60);
        let whole = |command| {
            event(
                1,
                SessionEvent::State {
                    state: SessionState {
                        command,
                        ..SessionState::default()
                    },
                },
            )
        };

        // A whole state never asks for anything, even one that holds a finished command.
        let mut attention = Attention::default();
        let done = Some(CommandState::Finished { exit: Some(1) });
        assert_eq!(
            attention.take(&whole(done.clone()), now, &ON, &around),
            None
        );
        // One that says no command runs drops the one that was seen begin.
        attention.take(&started(1), now, &ON, &around);
        attention.take(&whole(done), now, &ON, &around);
        assert_eq!(attention.take(&finished(1, None), end, &ON, &around), None);
        // One that says it still runs keeps it.
        attention.take(&started(1), now, &ON, &around);
        let running = Some(CommandState::Running { command: None });
        attention.take(&whole(running), now, &ON, &around);
        assert!(
            attention
                .take(&finished(1, None), end, &ON, &around)
                .is_some()
        );

        // Missed reports, or the program's end, drop it too.
        let mut attention = Attention::default();
        attention.take(&started(1), now, &ON, &around);
        attention.forget();
        assert_eq!(attention.take(&finished(1, None), end, &ON, &around), None);
        attention.take(&started(1), now, &ON, &around);
        let exit = News {
            key: key(1),
            what: SessionNews::Exit(ExitStatus::Code(0)),
        };
        assert_eq!(attention.take(&exit, now, &ON, &around), None);
        assert_eq!(attention.take(&finished(1, None), end, &ON, &around), None);
    }

    /// The application, with one window and the given settings, followed for attention.
    struct App {
        services: Arc<Services>,
        shell: Arc<crate::testing::FakeShell>,
        window: WindowId,
        key: SessionKey,
        _dir: crate::testing::TempDir,
    }

    async fn app(name: &str, settings: &str) -> App {
        use lume_layout::Store;
        use lume_session::{HostConfig, LocalBackend, Router};

        use crate::configsvc::ConfigService;
        use crate::marks::Marks;
        use crate::panes::{PaneStart, Panes};
        use crate::state::AppState;
        use crate::testing::{FakeShell, TempDir, size};
        use crate::tuning::WORKSPACE_NAME;

        let dir = TempDir::new(name);
        let shell = if cfg!(windows) {
            "shell = \"cmd.exe\"\nshell-args = [\"/d\"]"
        } else {
            "shell = \"/bin/sh\""
        };
        std::fs::create_dir_all(&dir.dirs().config).unwrap();
        std::fs::write(
            dir.dirs().config_file(),
            format!("[terminal]\n{shell}\n{settings}\n"),
        )
        .unwrap();
        let local = Arc::new(
            LocalBackend::in_process(HostConfig::default())
                .await
                .unwrap(),
        );
        let router = Router::new();
        router.insert(Router::local_id(), local.clone());
        let state = AppState::new(
            tokio::runtime::Handle::current(),
            router,
            vec![local],
            Duration::from_secs(600),
            Arc::new(Marks::start()),
        );
        let config = ConfigService::load(dir.dirs(), Some(WORKSPACE_NAME));
        let store = Store::new(dir.dirs().state_workspaces_dir());
        let (panes, _) =
            Panes::open(&state, Arc::clone(&config), store, WORKSPACE_NAME, None).await;
        state.set_panes(Arc::clone(&panes));
        let shell = Arc::new(FakeShell::default());
        panes.run(shell.clone());
        let window = panes.focused_window().unwrap();
        let pane = panes.workspace().panes()[0].id;
        let started = panes
            .start_sessions(
                window,
                vec![PaneStart {
                    pane,
                    size: size(80, 24),
                }],
            )
            .await;
        let started = &started.started[0];
        let key = SessionKey {
            backend: started.backend.clone(),
            session: started.session,
        };
        let registry = Arc::new(crate::builtins::registry());
        let plugins = crate::plugins::Plugins::new(
            config.dirs(),
            Arc::clone(&registry),
            Arc::clone(&state),
            Arc::clone(&config),
        );
        let services = Arc::new(Services {
            rex: crate::rex::Hosts::new(&config, &state, &panes, None),
            state,
            panes,
            config,
            registry,
            shell: shell.clone(),
            plugins,
        });
        watch(&services, shell.clone());
        App {
            services,
            shell,
            window,
            key,
            _dir: dir,
        }
    }

    async fn until(what: &str, done: impl Fn() -> bool) {
        let waiting = async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        if tokio::time::timeout(Duration::from_secs(20), waiting)
            .await
            .is_err()
        {
            panic!("{what} never happened");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_application_asks_for_attention_to_a_window_not_in_use() {
        let app = app(
            "attention",
            "notify-long-command = true\nlong-command-seconds = 5",
        )
        .await;
        let reports = app.services.state.reports();

        // The window the user is working in does not ask.
        app.shell.set_focused(app.window, true);
        reports.on_event(&app.key, &SessionEvent::Bell);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(app.shell.attention(), vec![]);

        app.shell.set_focused(app.window, false);
        reports.on_event(&app.key, &SessionEvent::Bell);
        let shell = Arc::clone(&app.shell);
        until("the window to ask for attention", || {
            !shell.attention().is_empty()
        })
        .await;
        assert_eq!(app.shell.attention(), vec![app.window]);

        // A command that ran long enough, named by what the pane runs.
        reports.on_event(&app.key, &SessionEvent::CommandStarted { command: None });
        tokio::time::sleep(Duration::from_millis(5100)).await;
        reports.on_event(&app.key, &SessionEvent::CommandFinished { exit: Some(4) });
        until("the notification", || !shell.notified().is_empty()).await;
        let shell_name = if cfg!(windows) { "cmd" } else { "sh" };
        assert_eq!(
            app.shell.notified(),
            vec![(
                "A command failed with exit code 4".to_owned(),
                format!("{shell_name}: it ran for 5 s"),
            )]
        );
        app.services.state.shutdown().await;
    }

    #[test]
    fn durations_are_said_as_a_person_says_them() {
        assert_eq!(spoken(Duration::from_secs(45)), "45 s");
        assert_eq!(spoken(Duration::from_secs(180)), "3 min");
        assert_eq!(spoken(Duration::from_secs(200)), "3 min 20 s");
        assert_eq!(spoken(Duration::from_secs(3600)), "1 h");
        assert_eq!(spoken(Duration::from_secs(3900 + 30)), "1 h 5 min");
    }
}
