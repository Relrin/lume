//! Who owns a layout.
//!
//! A layout has one owner, its authority. Everyone else asks the authority to apply an
//! intent and follows what it reports. [`LocalLayout`] is the authority for a
//! workspace this application owns: it keeps the workspace, applies intents one at a
//! time, numbers the results, and tells its followers.
//!
//! An intent that makes a terminal pane needs a session. The authority has the session
//! started by its [`Spawner`] first and applies the intent only once that has worked,
//! so a pane is never made for a session that could not be started.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use async_trait::async_trait;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::apply::{Started, apply, check, needs};
use crate::change::{Applied, Created, LayoutEvent, LayoutSnapshot};
use crate::error::{IntentError, LayoutError, SpawnFailure};
use crate::intent::{Intent, SpawnRequest};
use crate::model::Workspace;
use crate::tuning::CHANGE_BACKLOG;

/// Starts the sessions of new terminal panes.
#[async_trait]
pub trait Spawner: Send + Sync {
    /// Starts a session for a pane that is about to be made.
    async fn spawn(&self, request: &SpawnRequest) -> Result<Started, SpawnFailure>;

    /// Ends a session that was started for a pane that was then not made: the layout
    /// changed while the session was starting, and the intent no longer applies.
    async fn discard(&self, started: &Started);
}

/// Whoever owns a layout.
#[async_trait]
pub trait LayoutAuthority: Send + Sync {
    /// The layout as it is now.
    async fn view(&self) -> Result<LayoutSnapshot, LayoutError>;

    /// Applies an intent. Intents that only the application may apply are refused.
    async fn apply(&self, intent: Intent) -> Result<Applied, LayoutError>;

    /// What happens to the layout from now on, in order.
    fn changes(&self) -> ChangeStream;
}

/// Where the events of a [`ChangeStream`] come from.
#[async_trait]
pub trait ChangeSource: Send {
    /// The next event, or `None` once the authority is gone.
    async fn next(&mut self) -> Option<LayoutEvent>;
}

/// The events of a layout, for one follower.
pub struct ChangeStream {
    source: Box<dyn ChangeSource>,
}

impl ChangeStream {
    pub fn new(source: impl ChangeSource + 'static) -> Self {
        Self {
            source: Box::new(source),
        }
    }

    /// The next event, or `None` once the authority is gone.
    ///
    /// A follower that takes events more slowly than they happen is not sent what it
    /// missed: it is given the whole layout, and then the events after it.
    pub async fn next(&mut self) -> Option<LayoutEvent> {
        self.source.next().await
    }
}

struct State {
    workspace: Workspace,
    epoch: u32,
    revision: u64,
}

impl State {
    fn snapshot(&self) -> LayoutSnapshot {
        LayoutSnapshot {
            epoch: self.epoch,
            revision: self.revision,
            workspace: self.workspace.clone(),
        }
    }
}

/// The authority for a workspace this application owns.
pub struct LocalLayout {
    state: Mutex<State>,
    spawner: Arc<dyn Spawner>,
    events: broadcast::Sender<LayoutEvent>,
    /// This authority, for the followers it hands out, which ask it for the layout
    /// when they have fallen behind.
    me: Weak<LocalLayout>,
}

impl LocalLayout {
    /// An authority over `workspace`. `epoch` distinguishes this layout from those the
    /// same followers were shown before; it goes up each time the workspace is replaced.
    pub fn new(workspace: Workspace, epoch: u32, spawner: Arc<dyn Spawner>) -> Arc<Self> {
        let (events, _) = broadcast::channel(CHANGE_BACKLOG);
        Arc::new_cyclic(|me| Self {
            state: Mutex::new(State {
                workspace,
                epoch,
                revision: 0,
            }),
            spawner,
            events,
            me: me.clone(),
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The layout as it is now.
    pub fn snapshot(&self) -> LayoutSnapshot {
        self.state().snapshot()
    }

    /// The workspace as it is now.
    pub fn workspace(&self) -> Workspace {
        self.state().workspace.clone()
    }

    /// Puts another workspace in this one's place. A new epoch begins, and every
    /// follower is given the new layout whole.
    pub fn replace(&self, workspace: Workspace) -> LayoutSnapshot {
        let mut state = self.state();
        state.workspace = workspace;
        state.epoch = state.epoch.wrapping_add(1);
        state.revision = 0;
        let snapshot = state.snapshot();
        // Sent while the state is held, so events leave in the order they happened.
        let _ = self
            .events
            .send(LayoutEvent::Reset(Box::new(snapshot.clone())));
        snapshot
    }

    /// Applies an intent on the application's own behalf: one of those nobody else may
    /// apply, or any other.
    pub async fn apply_core(&self, intent: Intent) -> Result<Applied, LayoutError> {
        // Checked before a session is started, so that an intent that cannot be
        // applied starts nothing.
        let request = {
            let state = self.state();
            check(&state.workspace, &intent)?;
            needs(&state.workspace, &intent).cloned()
        };
        let started = match request {
            Some(request) => Some(
                self.spawner
                    .spawn(&request)
                    .await
                    .map_err(LayoutError::Spawn)?,
            ),
            None => None,
        };

        // The layout may have changed while the session was starting. The intent is
        // applied to the layout as it is now, and may be refused after all.
        let outcome = self.commit(intent, started.clone());
        if outcome.is_err()
            && let Some(started) = &started
        {
            self.spawner.discard(started).await;
        }
        Ok(outcome?)
    }

    fn commit(&self, intent: Intent, started: Option<Started>) -> Result<Applied, IntentError> {
        let mut state = self.state();
        let (changes, created): (_, Created) = apply(&mut state.workspace, intent, started)?;
        if !changes.is_empty() {
            state.revision += 1;
            // Nobody following is not a failure. Sent while the state is held, so
            // events leave in the order they happened.
            let _ = self.events.send(LayoutEvent::Changed {
                epoch: state.epoch,
                revision: state.revision,
                changes,
            });
        }
        Ok(Applied {
            epoch: state.epoch,
            revision: state.revision,
            created,
        })
    }
}

#[async_trait]
impl LayoutAuthority for LocalLayout {
    async fn view(&self) -> Result<LayoutSnapshot, LayoutError> {
        Ok(self.snapshot())
    }

    async fn apply(&self, intent: Intent) -> Result<Applied, LayoutError> {
        if intent.is_core_only() {
            return Err(IntentError::CoreOnly.into());
        }
        self.apply_core(intent).await
    }

    fn changes(&self) -> ChangeStream {
        ChangeStream::new(LocalChanges {
            events: self.events.subscribe(),
            layout: self.me.clone(),
            floor: None,
            waiting: VecDeque::new(),
        })
    }
}

/// One follower of a [`LocalLayout`].
struct LocalChanges {
    events: broadcast::Receiver<LayoutEvent>,
    layout: Weak<LocalLayout>,
    /// Where the layout stood when this follower was last given it whole. Events up to
    /// there are already in what it was given.
    floor: Option<(u32, u64)>,
    /// Events that happened while the follower was being brought up to date, which
    /// follow on from the layout it was given.
    waiting: VecDeque<LayoutEvent>,
}

/// Where in the sequence of a layout's events one stands.
fn place(event: &LayoutEvent) -> (u32, u64) {
    match event {
        LayoutEvent::Changed {
            epoch, revision, ..
        } => (*epoch, *revision),
        LayoutEvent::Reset(snapshot) => (snapshot.epoch, snapshot.revision),
    }
}

impl LocalChanges {
    /// Gives a follower that has missed events the layout as it is now. The events
    /// still waiting that led up to it are passed over here, so that the follower is
    /// level with the authority again and does not fall behind at its next event.
    fn catch_up(&mut self) -> Option<LayoutEvent> {
        let layout = self.layout.upgrade()?;
        let mut snapshot = layout.snapshot();
        self.waiting.clear();
        loop {
            match self.events.try_recv() {
                Ok(event) => {
                    if place(&event) > (snapshot.epoch, snapshot.revision) {
                        self.waiting.push_back(event);
                    }
                }
                // More went by unseen while this was catching up: begin again from
                // the layout as it is now.
                Err(TryRecvError::Lagged(_)) => {
                    snapshot = layout.snapshot();
                    self.waiting.clear();
                }
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
        self.floor = Some((snapshot.epoch, snapshot.revision));
        Some(LayoutEvent::Reset(Box::new(snapshot)))
    }
}

#[async_trait]
impl ChangeSource for LocalChanges {
    async fn next(&mut self) -> Option<LayoutEvent> {
        loop {
            if let Some(event) = self.waiting.pop_front() {
                return Some(event);
            }
            match self.events.recv().await {
                Ok(event) => {
                    let at = place(&event);
                    if self.floor.is_some_and(|floor| at <= floor) {
                        continue;
                    }
                    if matches!(event, LayoutEvent::Reset(_)) {
                        self.floor = Some(at);
                    }
                    return Some(event);
                }
                // The follower fell too far behind and events it had not taken are
                // gone.
                Err(RecvError::Lagged(_)) => return self.catch_up(),
                Err(RecvError::Closed) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use lume_proto::{BackendId, SessionId};

    use super::*;
    use crate::change::LayoutChange;
    use crate::ids::{PaneId, WindowId, WorkspaceId};
    use crate::intent::{Dir, NewContent};
    use crate::model::PaneContent;
    use crate::tuning::UNITS;

    /// Starts sessions that are numbers, and remembers what it was asked.
    #[derive(Default)]
    struct FakeSpawner {
        next: AtomicU64,
        fail: bool,
        discarded: Mutex<Vec<SessionId>>,
        /// Run while a session is "starting", to change the layout under the intent.
        meanwhile: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    #[async_trait]
    impl Spawner for FakeSpawner {
        async fn spawn(&self, request: &SpawnRequest) -> Result<Started, SpawnFailure> {
            if self.fail {
                return Err(SpawnFailure {
                    code: "spawn_failed".to_owned(),
                    message: "no such program".to_owned(),
                });
            }
            let meanwhile = self.meanwhile.lock().unwrap().take();
            if let Some(meanwhile) = meanwhile {
                meanwhile();
            }
            let number = self.next.fetch_add(1, Ordering::Relaxed) + 1;
            Ok(Started {
                backend: BackendId("local".to_owned()),
                session: SessionId::new(number).unwrap(),
                cwd: request.cwd.clone(),
            })
        }

        async fn discard(&self, started: &Started) {
            self.discarded.lock().unwrap().push(started.session);
        }
    }

    fn terminal() -> NewContent {
        NewContent::Terminal(SpawnRequest::default())
    }

    /// A layout with one window holding one pane, and the ids of both.
    async fn one_pane(spawner: Arc<FakeSpawner>) -> (Arc<LocalLayout>, WindowId, PaneId) {
        let workspace = Workspace::new(WorkspaceId::new(1).unwrap(), "test");
        let layout = LocalLayout::new(workspace, 1, spawner);
        let opened = layout
            .apply(Intent::WindowNew {
                content: terminal(),
                bounds: None,
            })
            .await
            .unwrap();
        (
            layout,
            opened.created.window.unwrap(),
            opened.created.pane.unwrap(),
        )
    }

    fn split(pane: PaneId) -> Intent {
        Intent::Split {
            pane,
            dir: Dir::Right,
            ratio: UNITS / 2,
            content: terminal(),
        }
    }

    fn session_of(layout: &LocalLayout, pane: PaneId) -> Option<SessionId> {
        match &layout.workspace().pane(pane)?.content {
            PaneContent::Terminal(terminal) => terminal.session,
            PaneContent::PluginView { .. } => None,
        }
    }

    #[tokio::test]
    async fn a_new_pane_gets_the_session_that_was_started_for_it() {
        let spawner = Arc::new(FakeSpawner::default());
        let (layout, _, first) = one_pane(spawner.clone()).await;
        // A window's first pane has no session yet: no page could say how large it is.
        assert_eq!(session_of(&layout, first), None);

        let mut changes = layout.changes();
        let applied = layout.apply(split(first)).await.unwrap();
        let second = applied.created.pane.unwrap();
        assert_eq!(session_of(&layout, second), SessionId::new(1));
        assert_eq!(applied.revision, 2);

        let Some(LayoutEvent::Changed {
            epoch,
            revision,
            changes,
        }) = changes.next().await
        else {
            panic!("the split was not reported");
        };
        assert_eq!((epoch, revision), (1, 2));
        assert!(
            changes
                .iter()
                .any(|change| matches!(change, LayoutChange::TabTree { .. }))
        );
    }

    #[tokio::test]
    async fn a_session_that_cannot_be_started_leaves_no_pane() {
        let spawner = Arc::new(FakeSpawner {
            fail: true,
            ..FakeSpawner::default()
        });
        let (layout, _, first) = one_pane(spawner).await;
        let before = layout.snapshot();

        let refused = layout.apply(split(first)).await.unwrap_err();
        assert_eq!(refused.code(), "spawn_failed");
        assert_eq!(layout.snapshot(), before);
    }

    #[tokio::test]
    async fn an_intent_that_cannot_be_applied_starts_no_session() {
        let spawner = Arc::new(FakeSpawner::default());
        let (layout, _, _) = one_pane(spawner.clone()).await;

        let refused = layout.apply(split(PaneId::new(999))).await.unwrap_err();
        assert_eq!(refused.code(), "not_found");
        assert_eq!(spawner.next.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn a_session_started_for_a_pane_that_went_away_is_ended() {
        let spawner = Arc::new(FakeSpawner::default());
        let (layout, _, first) = one_pane(spawner.clone()).await;

        // While the session starts, the pane that was to be split is closed.
        let closing = Arc::clone(&layout);
        *spawner.meanwhile.lock().unwrap() = Some(Box::new(move || {
            closing.commit(Intent::Close { pane: first }, None).unwrap();
        }));

        let refused = layout.apply(split(first)).await.unwrap_err();
        assert_eq!(refused.code(), "not_found");
        assert_eq!(
            *spawner.discarded.lock().unwrap(),
            vec![SessionId::new(1).unwrap()]
        );
        assert!(layout.workspace().panes().is_empty());
    }

    #[tokio::test]
    async fn only_the_application_may_record_a_session() {
        let spawner = Arc::new(FakeSpawner::default());
        let (layout, _, first) = one_pane(spawner).await;
        let record = Intent::SessionSet {
            pane: first,
            session: SessionId::new(7),
            backend: None,
            cwd: None,
        };

        let refused = layout.apply(record.clone()).await.unwrap_err();
        assert_eq!(refused, LayoutError::Intent(IntentError::CoreOnly));
        layout.apply_core(record).await.unwrap();
        assert_eq!(session_of(&layout, first), SessionId::new(7));
    }

    #[tokio::test]
    async fn an_intent_that_changes_nothing_is_not_reported() {
        let spawner = Arc::new(FakeSpawner::default());
        let (layout, window, _) = one_pane(spawner).await;
        let mut changes = layout.changes();

        // There is no pane to the left of the only pane.
        let applied = layout
            .apply(Intent::FocusDir {
                window,
                dir: Dir::Left,
            })
            .await
            .unwrap();
        assert_eq!(applied.revision, 1);

        layout
            .apply(Intent::PaneRename {
                pane: layout.workspace().panes()[0].id,
                title: Some("logs".to_owned()),
            })
            .await
            .unwrap();
        let Some(LayoutEvent::Changed { revision, .. }) = changes.next().await else {
            panic!("the rename was not reported");
        };
        assert_eq!(revision, 2);
    }

    #[tokio::test]
    async fn a_follower_that_falls_behind_is_given_the_layout_whole() {
        let spawner = Arc::new(FakeSpawner::default());
        let (layout, _, first) = one_pane(spawner).await;
        let mut changes = layout.changes();

        // More changes than a follower may fall behind by.
        let renames = CHANGE_BACKLOG as u64 + 10;
        for number in 0..renames {
            layout
                .apply(Intent::PaneRename {
                    pane: first,
                    title: Some(format!("name {number}")),
                })
                .await
                .unwrap();
        }

        let Some(LayoutEvent::Reset(snapshot)) = changes.next().await else {
            panic!("the follower was not given the layout");
        };
        assert_eq!(snapshot.revision, 1 + renames);
        assert_eq!(*snapshot, layout.snapshot());

        // What led up to the layout it was given is passed over; what comes after
        // follows on from it.
        layout
            .apply(Intent::PaneRename {
                pane: first,
                title: None,
            })
            .await
            .unwrap();
        let Some(LayoutEvent::Changed { revision, .. }) = changes.next().await else {
            panic!("the change after the layout was not reported");
        };
        assert_eq!(revision, snapshot.revision + 1);
    }

    #[tokio::test]
    async fn replacing_the_workspace_begins_a_new_epoch() {
        let spawner = Arc::new(FakeSpawner::default());
        let (layout, _, _) = one_pane(spawner).await;
        let mut changes = layout.changes();

        let other = Workspace::new(WorkspaceId::new(2).unwrap(), "other");
        let snapshot = layout.replace(other.clone());
        assert_eq!((snapshot.epoch, snapshot.revision), (2, 0));
        assert_eq!(layout.workspace(), other);
        assert_eq!(
            changes.next().await,
            Some(LayoutEvent::Reset(Box::new(snapshot)))
        );
    }

    #[tokio::test]
    async fn the_changes_end_when_the_authority_is_gone() {
        let spawner = Arc::new(FakeSpawner::default());
        let (layout, _, _) = one_pane(spawner).await;
        let mut changes = layout.changes();
        drop(layout);
        assert_eq!(changes.next().await, None);
    }
}
