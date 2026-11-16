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

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use lume_proto::app::SessionNews;
use lume_session::{ExitStatus, Launcher, SessionEvent, SessionState};
use tokio::sync::broadcast;

use crate::carrier::SessionKey;
use crate::tuning::SESSION_NEWS_BACKLOG;

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
    state: SessionState,
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

    /// Notes what a session that was just started runs.
    pub fn started(&self, key: SessionKey, launcher: Launcher) {
        self.sessions().entry(key).or_default().launcher = Some(launcher);
    }

    /// Takes in a report on its way to the page.
    pub fn on_event(&self, key: &SessionKey, event: &SessionEvent) {
        self.sessions()
            .entry(key.clone())
            .or_default()
            .state
            .apply(event);
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

    /// What a session's reports add up to so far.
    #[cfg(test)]
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
    use lume_session::{BackendId, SessionId};

    use super::*;

    fn key(raw: u64) -> SessionKey {
        SessionKey {
            backend: BackendId("local".to_owned()),
            session: SessionId::new(raw).unwrap(),
        }
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
        let states = SessionStates::new();
        assert_eq!(states.state(&key(1)), None);
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
    }

    #[test]
    fn a_directory_is_only_known_with_what_the_session_runs() {
        let states = SessionStates::new();
        states.on_event(&key(1), &cwd("file://box/a"));
        // A session the application did not start: nothing says what it runs.
        assert_eq!(states.reported_dir(&key(1)), None);
        states.started(key(1), shell());
        assert_eq!(
            states.reported_dir(&key(1)),
            Some((shell(), "file://box/a".to_owned()))
        );
        states.started(key(2), shell());
        assert_eq!(states.reported_dir(&key(2)), None);
    }

    #[tokio::test]
    async fn a_follower_gets_the_states_so_far_and_then_every_report() {
        let states = SessionStates::new();
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
