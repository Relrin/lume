//! A server's layout, for the application to show and follow.
//!
//! A server keeps a layout for each of its sessions. The application shows one layout
//! of a backend at a time, so a backend has one session on show, and [`RexLayout`] is
//! the authority over that session's layout as the application knows authorities.
//!
//! It holds a layout of its own that is a copy of the server's: each view the backend
//! reads of the session on show is turned into a workspace and put in the copy's
//! place, as a step that says what differs. Another session on show is another
//! layout, given whole. With no session on show, or once the session is gone, the
//! layout is one window with nothing in it.
//!
//! A view that cannot be shown changes nothing: the copy stays as it was, and asking
//! for the layout says what is wrong until a view comes that can be shown.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use lume_layout::{
    Applied, ChangeSource, ChangeStream, Created, Intent, LayoutAuthority, LayoutError,
    LayoutEvent, LayoutSnapshot, LocalLayout, PaneId, SpawnFailure, SpawnRequest, Spawner, Started,
    validate,
};
use lume_proto::BackendId;

use crate::api::View;
use crate::backend::{Link, Seen, Shared};
use crate::control::Control;
use crate::error::RexError;
use crate::intents::{Makes, plan};
use crate::layout::{LayoutIds, Names, translate};
use crate::tuning::APPLY_SETTLE;

/// The first of the layouts a backend gives. Each other session put on show is the
/// next.
const FIRST_EPOCH: u32 = 1;

/// Starts no sessions. The copy of a server's layout is never asked to make a pane:
/// panes are made by the server.
struct NoSessions;

#[async_trait]
impl Spawner for NoSessions {
    async fn spawn(&self, _request: &SpawnRequest) -> Result<Started, SpawnFailure> {
        Err(SpawnFailure {
            code: "unsupported".to_owned(),
            message: "a pane of a server's layout is made by the server".to_owned(),
        })
    }

    async fn discard(&self, _started: &Started) {}
}

/// The changes of a layout whose backend is gone: there are none.
struct Ended;

#[async_trait]
impl ChangeSource for Ended {
    async fn next(&mut self) -> Option<LayoutEvent> {
        None
    }
}

/// A session and the revision of a view of it.
type Stand = Option<(String, u64)>;

struct Held {
    /// The copy of the server's layout. None once the backend is gone, which ends
    /// what its followers are sent.
    layout: Option<Arc<LocalLayout>>,
    ids: LayoutIds,
    /// What the server calls the things of the copy.
    names: Names,
    /// The session the application asked to be shown.
    on_show: Option<String>,
    /// The session the copy is of. None while it is the empty window.
    current: Option<String>,
    /// The view that was last tried: the one the copy was made from, or the one that
    /// could not be shown.
    tried: Stand,
    /// Why the view that was last tried could not be shown, if it could not.
    fault: Option<String>,
}

/// The authority over the layout of a backend's session on show.
pub(crate) struct RexLayout {
    shared: Arc<Shared>,
    backend: BackendId,
    held: Mutex<Held>,
}

/// The view of a session with nothing in it, which no session on show is shown as.
fn nothing() -> View {
    View {
        revision: 0,
        session_id: None,
        label: String::new(),
        active_window_id: None,
        windows: Vec::new(),
    }
}

impl RexLayout {
    pub(crate) fn new(shared: Arc<Shared>, backend: BackendId) -> Arc<Self> {
        let mut ids = LayoutIds::default();
        // The view of nothing is a workspace, whatever numbers sessions are given.
        let empty = translate(&nothing(), &mut ids, &backend, &mut |_| {
            unreachable!("nothing has no blocks")
        })
        .expect("nothing is shown as an empty window");
        Arc::new(Self {
            shared,
            backend,
            held: Mutex::new(Held {
                layout: Some(LocalLayout::new(
                    empty.workspace,
                    FIRST_EPOCH,
                    Arc::new(NoSessions),
                )),
                ids,
                names: empty.names,
                on_show: None,
                current: None,
                tried: None,
                fault: None,
            }),
        })
    }

    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The session on show.
    pub(crate) fn on_show(&self) -> Option<String> {
        self.held().on_show.clone()
    }

    /// Puts a session on show, or none. The session must have been opened. If its
    /// view cannot be shown, what was on show stays.
    pub(crate) fn show(&self, session: Option<&str>) -> Result<(), RexError> {
        let mut held = self.held();
        let before = (held.on_show.clone(), held.tried.clone(), held.fault.clone());
        let same = before.0.as_deref() == session;
        held.on_show = session.map(str::to_owned);
        self.publish(&mut held);
        // The view of nothing is always shown, so a fault here is the session's.
        let Some(fault) = held.fault.clone().filter(|_| session.is_some()) else {
            return Ok(());
        };
        if !same {
            // Nothing but these was changed by a view that could not be shown.
            (held.on_show, held.tried, held.fault) = before;
        }
        Err(RexError::Protocol(fault))
    }

    /// Brings the copy up to the view the backend holds of the session on show.
    pub(crate) fn refresh(&self) {
        self.publish(&mut self.held());
    }

    /// Ends the layout: the backend is gone. Those who follow it are told so.
    pub(crate) fn close(&self) {
        self.held().layout = None;
    }

    fn publish(&self, held: &mut Held) {
        let Some(layout) = held.layout.clone() else {
            return;
        };
        let seen = match &held.on_show {
            Some(session) => self.shared.seen(session),
            None => Seen::Gone,
        };
        let (session, view) = match seen {
            // Opened and not read yet: there is a view to come.
            Seen::Unread => return,
            // No longer a session of the server's, or let go of here.
            Seen::Gone => {
                held.on_show = None;
                (None, nothing())
            }
            Seen::Read(view) => (held.on_show.clone(), view),
        };
        let stand = session.clone().map(|session| (session, view.revision));
        if held.tried == stand && (held.current == session || held.fault.is_some()) {
            return;
        }
        held.tried = stand;

        let shared = &self.shared;
        let of = session.as_deref().unwrap_or_default();
        let made = translate(&view, &mut held.ids, &self.backend, &mut |block| {
            shared.session_number(of, block)
        })
        .map_err(|error| error.to_string())
        .and_then(|shown| match validate(&shown.workspace) {
            Ok(()) => Ok(shown),
            Err(wrong) => Err(format!(
                "a server's layout cannot be shown: {}",
                wrong.first().map(ToString::to_string).unwrap_or_default()
            )),
        });
        match made {
            Ok(shown) => {
                if held.current == session {
                    layout.update(|_| Some(shown.workspace));
                } else {
                    layout.replace(shown.workspace);
                }
                held.names = shown.names;
                held.current = session;
                held.fault = None;
            }
            Err(fault) => {
                tracing::warn!(server = %shared.name, %fault, "a session's layout is not shown");
                held.fault = Some(fault);
            }
        }
    }
}

#[async_trait]
impl LayoutAuthority for RexLayout {
    async fn view(&self) -> Result<LayoutSnapshot, LayoutError> {
        let mut held = self.held();
        self.publish(&mut held);
        let Some(layout) = &held.layout else {
            return Err(LayoutError::Closed);
        };
        match &held.fault {
            Some(fault) => Err(LayoutError::Unsupported(fault.clone())),
            None => Ok(layout.snapshot()),
        }
    }

    /// Asks the server for what the intent means, and answers once the copy shows what
    /// the server made of it.
    async fn apply(&self, intent: Intent) -> Result<Applied, LayoutError> {
        let (session, plan) = {
            let mut held = self.held();
            self.publish(&mut held);
            let Some(layout) = &held.layout else {
                return Err(LayoutError::Closed);
            };
            let Some(session) = held.current.clone() else {
                return Err(LayoutError::Unsupported(
                    "no session of the Rex server is on show".to_owned(),
                ));
            };
            let plan = plan(&intent, &layout.workspace(), &held.names)?;
            (session, plan)
        };
        let control = self.shared.control().map_err(refusal)?;

        let mut revision = 0;
        let mut made = None;
        for call in plan.calls {
            let answer = control
                .call(call.method, call.args)
                .await
                .map_err(refusal)?;
            let reached = answer.payload["revision"].as_u64().unwrap_or(0);
            revision = revision.max(reached);
            if plan.makes == Makes::Pane {
                made = answer.payload["block_ids"][0].as_str().map(str::to_owned);
            }
        }
        self.settle(&control, &session, revision).await;

        let mut held = self.held();
        self.publish(&mut held);
        let Some(layout) = &held.layout else {
            return Err(LayoutError::Closed);
        };
        let snapshot = layout.snapshot();
        let created = Created {
            // The pane of the block the server made, as the copy now numbers it.
            pane: made
                .and_then(|block| held.ids.known(&block))
                .map(PaneId::new)
                .filter(|pane| held.names.panes.contains_key(pane)),
            ..Created::default()
        };
        Ok(Applied {
            epoch: snapshot.epoch,
            revision: snapshot.revision,
            created,
        })
    }

    fn changes(&self) -> ChangeStream {
        match &self.held().layout {
            Some(layout) => layout.changes(),
            None => ChangeStream::new(Ended),
        }
    }
}

impl RexLayout {
    /// Waits until the backend holds a view of the session at `revision` or later:
    /// what a call changed is then in the copy. The server says that a view changed
    /// before it answers the call that changed it, and the backend reads the view when
    /// it is told; if that does not happen, the view is read here.
    async fn settle(&self, control: &Control, session: &str, revision: u64) {
        let mut changed = self.shared.changed();
        // A session that is no longer opened has no view to wait for.
        let reached = || {
            self.shared
                .revision_of(session)
                .is_none_or(|held| held >= revision)
        };
        let told = async {
            while !reached() {
                if changed.changed().await.is_err() {
                    break;
                }
            }
        };
        if tokio::time::timeout(APPLY_SETTLE, told).await.is_err()
            && let Err(error) = self.shared.read_view(control, session).await
        {
            tracing::debug!(server = %self.shared.name, %error, "a session's view could not be read");
        }
    }
}

/// Why a server did not do what it was asked, as a layout's owner says it.
fn refusal(error: RexError) -> LayoutError {
    match error {
        RexError::Disconnected | RexError::Closed { .. } => LayoutError::Closed,
        RexError::Call { code, message, .. } => LayoutError::Refused { code, message },
        other => LayoutError::Refused {
            code: other.code().to_owned(),
            message: other.to_string(),
        },
    }
}

/// Keeps a backend's layout level with what the backend knows, until the backend can
/// no longer be used.
pub(crate) async fn follow(layout: Arc<RexLayout>) {
    let mut changed = layout.shared.changed();
    let mut link = layout.shared.link();
    loop {
        layout.refresh();
        tokio::select! {
            told = changed.changed() => {
                if told.is_err() {
                    break;
                }
            }
            told = link.changed() => {
                let over = matches!(*link.borrow_and_update(), Link::Refused { .. } | Link::Closed);
                if told.is_err() || over {
                    break;
                }
            }
        }
    }
    layout.close();
}
