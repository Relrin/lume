//! What the application keeps while it runs: the session backends, each window's
//! carrier, and which carrier is showing which session.
//!
//! A page can go away at any time: it is reloaded, or its window closes. The sessions it
//! was showing do not go with it at once. Its carrier is retired, its sessions are left
//! without a holder, and a page that opens a carrier within the grace period may attach
//! to them again. Sessions nobody claims in that time are ended, so a program is never
//! left running with no window to show it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use lume_proto::HydrateMode;
use lume_session::{
    BackendCaps, BackendId, ClientFacts, Hydrate, LocalBackend, Router, SessionId, SessionInfo,
    SpawnSpec, TermSize,
};
use serde::Serialize;
use tokio::runtime::Handle;
use tokio::task::{AbortHandle, JoinHandle};

use crate::carrier::{Carrier, Link, OnExit, SessionKey};
use crate::error::CommandError;

/// Who is responsible for a session.
enum Holder {
    /// The carrier with this id spawned it or is attached to it.
    Held(u32),
    /// Its carrier went away. Unless another claims it, the timer ends it.
    Orphan { token: u64, timer: AbortHandle },
}

type Sessions = Arc<Mutex<HashMap<SessionKey, Holder>>>;

/// What attaching to a session tells the view.
#[derive(Debug, Serialize)]
pub struct Attached {
    /// The terminal's size when the attachment opened.
    pub size: TermSize,
    /// Whether the view's terminal should pull lines back out of scrollback when it
    /// grows taller.
    pub resize_pulls_scrollback: bool,
    /// Whether a frame with the state to start from precedes the output.
    pub hydrate: HydrateMode,
    /// What the session's backend can do.
    pub caps: BackendCaps,
}

/// One session and the backend it runs on.
#[derive(Debug, Serialize)]
pub struct Listed {
    pub backend: BackendId,
    pub info: SessionInfo,
}

pub struct AppState {
    runtime: Handle,
    router: Router,
    /// The backend for sessions on this machine, when the application runs one.
    local: Option<Arc<LocalBackend>>,
    /// The carrier of each webview, by the webview's label.
    links: Mutex<HashMap<String, Arc<Link>>>,
    sessions: Sessions,
    /// Links that are detaching their streams. A new carrier waits for these, so that
    /// its attach requests reach the backends after the detaches.
    retiring: Mutex<Vec<JoinHandle<()>>>,
    next_carrier: AtomicU32,
    next_orphan: AtomicU64,
    grace: Duration,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AppState {
    /// `grace` is how long a session waits for a new holder once its carrier is gone.
    pub fn new(
        runtime: Handle,
        router: Router,
        local: Option<Arc<LocalBackend>>,
        grace: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            router,
            local,
            links: Mutex::new(HashMap::new()),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            retiring: Mutex::new(Vec::new()),
            next_carrier: AtomicU32::new(1),
            next_orphan: AtomicU64::new(1),
            grace,
        })
    }

    /// Replaces the carrier of the webview called `label` and returns the new one's id,
    /// which the webview sends back with every request.
    ///
    /// The carrier it replaces is retired first, and this returns only once its streams
    /// are detached: the page that asked can attach to the same sessions straight away.
    pub async fn open(self: &Arc<Self>, label: &str, carrier: Arc<dyn Carrier>) -> u32 {
        self.retire(label);
        let retiring = std::mem::take(&mut *lock(&self.retiring));
        for detaching in retiring {
            let _ = detaching.await;
        }

        let id = self.next_carrier.fetch_add(1, Ordering::Relaxed);
        let sessions = Arc::clone(&self.sessions);
        let on_exit: OnExit = Arc::new(move |key| {
            // A session whose program has ended needs no holder and no timer.
            if let Some(Holder::Orphan { timer, .. }) = lock(&sessions).remove(key) {
                timer.abort();
            }
        });
        let link = Link::open(id, carrier, &self.runtime, on_exit);
        let replaced = lock(&self.links).insert(label.to_owned(), link);
        if let Some(replaced) = replaced {
            // Another open for the same webview finished in between; the newest wins.
            self.retire_link(replaced);
        }
        id
    }

    /// Retires the carrier of the webview called `label`, if it has one: its page is
    /// gone. Does not wait; safe to call from outside the runtime.
    pub fn retire(self: &Arc<Self>, label: &str) {
        let link = lock(&self.links).remove(label);
        if let Some(link) = link {
            self.retire_link(link);
        }
    }

    fn retire_link(self: &Arc<Self>, link: Arc<Link>) {
        let held: Vec<SessionKey> = lock(&self.sessions)
            .iter()
            .filter(|(_, holder)| matches!(holder, Holder::Held(id) if *id == link.id()))
            .map(|(key, _)| key.clone())
            .collect();
        for key in held {
            self.orphan(key);
        }
        let detaching = self.runtime.spawn(async move { link.retire().await });
        lock(&self.retiring).push(detaching);
    }

    /// The carrier a request names, if it is still the webview's current one.
    fn link(&self, label: &str, carrier: u32) -> Result<Arc<Link>, CommandError> {
        let links = lock(&self.links);
        let link = links.get(label).ok_or_else(CommandError::no_carrier)?;
        if link.id() == carrier {
            Ok(Arc::clone(link))
        } else {
            Err(CommandError::stale_carrier())
        }
    }

    /// Makes `carrier` the holder of a session, or leaves the session to the grace
    /// timer if that carrier has been replaced in the meantime.
    fn claim(self: &Arc<Self>, label: &str, carrier: u32, key: SessionKey) {
        {
            // Held across both: a carrier cannot be retired between the check and the
            // claim, which would leave the session held by nobody for ever.
            let links = lock(&self.links);
            if links.get(label).is_some_and(|link| link.id() == carrier) {
                let previous = lock(&self.sessions).insert(key, Holder::Held(carrier));
                if let Some(Holder::Orphan { timer, .. }) = previous {
                    timer.abort();
                }
                return;
            }
        }
        self.orphan(key);
    }

    /// Leaves a session without a holder and starts the timer that ends it.
    fn orphan(self: &Arc<Self>, key: SessionKey) {
        let token = self.next_orphan.fetch_add(1, Ordering::Relaxed);
        let state = Arc::downgrade(self);
        let waiting = key.clone();
        let grace = self.grace;
        let timer = self
            .runtime
            .spawn(async move {
                tokio::time::sleep(grace).await;
                if let Some(state) = state.upgrade() {
                    state.end_orphan(&waiting, token).await;
                }
            })
            .abort_handle();
        let previous = lock(&self.sessions).insert(key, Holder::Orphan { token, timer });
        if let Some(Holder::Orphan { timer, .. }) = previous {
            timer.abort();
        }
    }

    /// Ends a session whose grace period ran out, unless it has been claimed since.
    async fn end_orphan(&self, key: &SessionKey, token: u64) {
        {
            let mut sessions = lock(&self.sessions);
            match sessions.get(key) {
                Some(Holder::Orphan { token: current, .. }) if *current == token => {
                    sessions.remove(key);
                }
                _ => return,
            }
        }
        tracing::info!(session = %key.session, "ending a session no page came back for");
        if let Ok(backend) = self.router.get(&key.backend)
            && let Err(error) = backend.kill(key.session).await
        {
            // Usually the program had already ended.
            tracing::debug!(session = %key.session, %error, "the session could not be ended");
        }
    }

    /// The id of the backend new sessions start on.
    pub fn default_backend(&self) -> BackendId {
        Router::local_id()
    }

    /// Starts a session for the page behind `carrier`.
    pub async fn spawn(
        self: &Arc<Self>,
        label: &str,
        carrier: u32,
        backend: &BackendId,
        spec: SpawnSpec,
    ) -> Result<SessionId, CommandError> {
        self.link(label, carrier)?;
        let session = self.router.get(backend)?.spawn(spec).await?;
        let key = SessionKey {
            backend: backend.clone(),
            session,
        };
        self.claim(label, carrier, key);
        Ok(session)
    }

    /// Attaches the page behind `carrier` to a session, on a stream id the page chose.
    pub async fn attach(
        self: &Arc<Self>,
        label: &str,
        carrier: u32,
        backend: &BackendId,
        session: SessionId,
        stream: u32,
        facts: ClientFacts,
    ) -> Result<Attached, CommandError> {
        let link = self.link(label, carrier)?;
        link.reserve(stream)?;
        let backend_id = backend;
        let backend = self.router.get(backend_id)?;
        let attachment = backend.attach(session, facts).await?;
        let attached = Attached {
            size: attachment.size,
            resize_pulls_scrollback: attachment.resize_pulls_scrollback,
            hydrate: match attachment.hydrate {
                Hydrate::None => HydrateMode::None,
                Hydrate::Snapshot { .. } => HydrateMode::Snapshot,
                Hydrate::VtReplay(_) => HydrateMode::VtReplay,
            },
            caps: backend.caps(),
        };
        let key = SessionKey {
            backend: backend_id.clone(),
            session,
        };
        link.bind(stream, key.clone(), attachment).await?;
        self.claim(label, carrier, key);
        Ok(attached)
    }

    /// Ends a session.
    pub async fn kill(
        &self,
        label: &str,
        carrier: u32,
        backend: &BackendId,
        session: SessionId,
    ) -> Result<(), CommandError> {
        self.link(label, carrier)?;
        let key = SessionKey {
            backend: backend.clone(),
            session,
        };
        if let Some(Holder::Orphan { timer, .. }) = lock(&self.sessions).remove(&key) {
            timer.abort();
        }
        Ok(self.router.get(backend)?.kill(session).await?)
    }

    /// Every session of every backend.
    pub async fn list(&self, label: &str, carrier: u32) -> Result<Vec<Listed>, CommandError> {
        self.link(label, carrier)?;
        let mut listed = Vec::new();
        for backend in self.router.ids() {
            for info in self.router.get(&backend)?.list().await? {
                listed.push(Listed {
                    backend: backend.clone(),
                    info,
                });
            }
        }
        Ok(listed)
    }

    /// Acts on the frames a page sent on its carrier.
    pub async fn send(&self, label: &str, carrier: u32, frames: &[u8]) -> Result<(), CommandError> {
        let link = self.link(label, carrier)?;
        link.apply(frames).await
    }

    /// Retires every carrier and ends every session. Call before the process exits: a
    /// program in a terminal can outlive a process that exits without closing it.
    pub async fn shutdown(self: &Arc<Self>) {
        let labels: Vec<String> = lock(&self.links).keys().cloned().collect();
        for label in labels {
            self.retire(&label);
        }
        for (_, holder) in lock(&self.sessions).drain() {
            if let Holder::Orphan { timer, .. } = holder {
                timer.abort();
            }
        }
        let retiring = std::mem::take(&mut *lock(&self.retiring));
        for detaching in retiring {
            let _ = detaching.await;
        }
        if let Some(local) = &self.local {
            local.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use lume_proto::{Body, Message};
    use lume_session::{ExitStatus, HostConfig};
    use tokio::time::{sleep, timeout};

    use super::*;
    use crate::testing::{
        CREDIT, PRINT_MARK, RecordingCarrier, STEP, client_frames, facts, shell, size,
    };

    const WINDOW: &str = "main";
    const MARK: &str = "lume-k7q-done";

    /// A long grace, for tests in which no session should be ended by it.
    const PATIENT: Duration = Duration::from_secs(600);

    async fn local_state(grace: Duration) -> Arc<AppState> {
        let local = Arc::new(
            LocalBackend::in_process(HostConfig::default())
                .await
                .unwrap(),
        );
        let router = Router::new();
        router.insert(Router::local_id(), local.clone());
        AppState::new(Handle::current(), router, Some(local), grace)
    }

    fn local() -> BackendId {
        Router::local_id()
    }

    /// The output a carrier has received for one stream, as text.
    fn output(carrier: &RecordingCarrier, stream: u32) -> String {
        let mut text = String::new();
        for message in carrier.messages() {
            if let (true, Body::Data(bytes)) = (message.stream == stream, &message.body) {
                text.push_str(&String::from_utf8_lossy(bytes));
            }
        }
        text
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        let waiting = async {
            while !done() {
                sleep(Duration::from_millis(10)).await;
            }
        };
        if timeout(STEP, waiting).await.is_err() {
            panic!("{what} never happened");
        }
    }

    async fn until_listed(state: &Arc<AppState>, carrier: u32, count: usize) {
        let waiting = async {
            while state.list(WINDOW, carrier).await.unwrap().len() != count {
                sleep(Duration::from_millis(20)).await;
            }
        };
        if timeout(STEP, waiting).await.is_err() {
            panic!("the session list never had {count} entries");
        }
    }

    /// Opens a carrier, starts a shell and attaches to it on `stream`.
    async fn started(
        state: &Arc<AppState>,
        stream: u32,
    ) -> (Arc<RecordingCarrier>, u32, SessionId) {
        let carrier = RecordingCarrier::new();
        let id = state.open(WINDOW, carrier.clone()).await;
        let session = state.spawn(WINDOW, id, &local(), shell()).await.unwrap();
        state
            .attach(WINDOW, id, &local(), session, stream, facts())
            .await
            .unwrap();
        (carrier, id, session)
    }

    async fn send(state: &Arc<AppState>, carrier: u32, messages: &[Message]) {
        state
            .send(WINDOW, carrier, &client_frames(messages))
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn output_reaches_the_carrier_under_the_views_stream_id_once_credit_is_granted() {
        let state = local_state(PATIENT).await;
        let carrier = RecordingCarrier::new();
        let id = state.open(WINDOW, carrier.clone()).await;
        let session = state.spawn(WINDOW, id, &local(), shell()).await.unwrap();
        let attached = state
            .attach(WINDOW, id, &local(), session, 5, facts())
            .await
            .unwrap();
        assert_eq!(attached.size, size(80, 24));
        assert_eq!(attached.hydrate, HydrateMode::None);
        assert_eq!(attached.resize_pulls_scrollback, cfg!(unix));

        // A shell prints a prompt as soon as it starts, and the command below prints
        // more. Without credit none of it may arrive.
        send(
            &state,
            id,
            &[Message::new(5, Body::Input(PRINT_MARK.to_vec()))],
        )
        .await;
        sleep(Duration::from_millis(300)).await;
        assert_eq!(carrier.messages(), vec![]);

        send(&state, id, &[Message::new(5, Body::Credit(CREDIT))]).await;
        until("the command's output", || {
            output(&carrier, 5).contains(MARK)
        })
        .await;
        assert!(carrier.messages().iter().all(|message| message.stream == 5));

        state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_resize_is_marked_in_the_output_and_an_exit_ends_it() {
        let state = local_state(PATIENT).await;
        let (carrier, id, session) = started(&state, 1).await;
        let resize = Body::Resize {
            size: size(100, 30),
            cell_px: facts().cell_px,
        };
        send(
            &state,
            id,
            &[
                Message::new(1, Body::Credit(CREDIT)),
                Message::new(1, resize),
                Message::new(1, Body::Facts(facts())),
            ],
        )
        .await;
        until("the resize mark", || {
            carrier
                .messages()
                .contains(&Message::new(1, Body::Resized(size(100, 30))))
        })
        .await;

        state.kill(WINDOW, id, &local(), session).await.unwrap();
        until("the exit", || {
            matches!(
                carrier.messages().last(),
                Some(Message {
                    stream: 1,
                    body: Body::Exit(_)
                })
            )
        })
        .await;
        until_listed(&state, id, 0).await;
        // A session that has ended has no holder left to time out.
        until("the session to be forgotten", || {
            lock(&state.sessions).is_empty()
        })
        .await;

        state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn frames_for_no_stream_are_dropped_and_malformed_ones_refused() {
        let state = local_state(PATIENT).await;
        let (_carrier, id, _session) = started(&state, 1).await;

        send(&state, id, &[Message::new(99, Body::Input(b"x".to_vec()))]).await;

        let frames = client_frames(&[Message::new(1, Body::Input(b"hello".to_vec()))]);
        let cut = state.send(WINDOW, id, &frames[..frames.len() - 1]).await;
        assert_eq!(cut.unwrap_err().code, "bad_request");

        // Granting more than a stream may hold closes the stream, not the carrier.
        let greedy = client_frames(&[Message::new(1, Body::Credit(u32::MAX))]);
        let refused = state.send(WINDOW, id, &greedy).await;
        assert_eq!(refused.unwrap_err().code, "limit_exceeded");
        send(&state, id, &[Message::new(1, Body::Input(b"x".to_vec()))]).await;

        state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stream_id_is_never_used_twice() {
        let state = local_state(PATIENT).await;
        let (_carrier, id, session) = started(&state, 5).await;
        send(&state, id, &[Message::new(5, Body::Detach)]).await;

        for stream in [0, 4, 5] {
            let again = state
                .attach(WINDOW, id, &local(), session, stream, facts())
                .await;
            assert_eq!(again.unwrap_err().code, "invalid_stream", "stream {stream}");
        }
        state
            .attach(WINDOW, id, &local(), session, 6, facts())
            .await
            .unwrap();

        state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_new_page_can_attach_to_the_sessions_of_the_one_it_replaced() {
        let state = local_state(Duration::from_millis(600)).await;
        let (old_carrier, old, session) = started(&state, 1).await;

        // The page reloads: no detach is sent, a new carrier just opens.
        let carrier = RecordingCarrier::new();
        let id = state.open(WINDOW, carrier.clone()).await;
        assert_ne!(id, old);

        // The old page's requests are refused from here on.
        let stale = state.send(WINDOW, old, &[]).await;
        assert_eq!(stale.unwrap_err().code, "stale_carrier");
        let stale = state.list(WINDOW, old).await;
        assert_eq!(stale.unwrap_err().code, "stale_carrier");
        let nowhere = state.list("other", id).await;
        assert_eq!(nowhere.unwrap_err().code, "no_carrier");

        // Stream ids start again with the new carrier.
        state
            .attach(WINDOW, id, &local(), session, 1, facts())
            .await
            .unwrap();
        send(
            &state,
            id,
            &[
                Message::new(1, Body::Credit(CREDIT)),
                Message::new(1, Body::Input(PRINT_MARK.to_vec())),
            ],
        )
        .await;
        until("output on the new carrier", || {
            output(&carrier, 1).contains(MARK)
        })
        .await;
        assert_eq!(output(&old_carrier, 1), "");

        // Claimed again, the session outlives the grace period.
        sleep(Duration::from_millis(1200)).await;
        assert_eq!(state.list(WINDOW, id).await.unwrap().len(), 1);

        state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sessions_no_page_comes_back_for_are_ended() {
        let state = local_state(Duration::from_millis(200)).await;
        let (_carrier, id, _session) = started(&state, 1).await;
        // A second session that was started and never attached to.
        state.spawn(WINDOW, id, &local(), shell()).await.unwrap();
        until_listed(&state, id, 2).await;

        state.retire(WINDOW);

        let observer = state.open("observer", RecordingCarrier::new()).await;
        let waiting = async {
            while !state.list("observer", observer).await.unwrap().is_empty() {
                sleep(Duration::from_millis(20)).await;
            }
        };
        timeout(STEP, waiting)
            .await
            .expect("the abandoned sessions were never ended");
        assert!(lock(&state.sessions).is_empty());

        state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutting_down_ends_every_session() {
        let state = local_state(PATIENT).await;
        let (carrier, id, _session) = started(&state, 1).await;
        send(&state, id, &[Message::new(1, Body::Credit(CREDIT))]).await;

        state.shutdown().await;

        // Nothing is delivered to a retired carrier, and the backend is closed.
        let before = carrier.batches().len();
        sleep(Duration::from_millis(100)).await;
        assert_eq!(carrier.batches().len(), before);
        let observer = state.open("observer", RecordingCarrier::new()).await;
        let listed = state.list("observer", observer).await;
        assert_eq!(listed.unwrap_err().code, "disconnected");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_view_is_told_when_its_session_ends_without_an_exit() {
        let state = local_state(PATIENT).await;
        let (carrier, _id, _session) = started(&state, 1).await;

        // The backend goes away under the carrier.
        state.local.as_ref().unwrap().shutdown().await;

        until("the exit", || {
            carrier
                .messages()
                .iter()
                .any(|message| matches!(message.body, Body::Exit(_)))
        })
        .await;
        let exits: Vec<Message> = carrier
            .messages()
            .into_iter()
            .filter(|message| matches!(message.body, Body::Exit(_)))
            .collect();
        assert_eq!(exits.len(), 1);
        assert!(matches!(
            exits[0].body,
            Body::Exit(ExitStatus::Unknown | ExitStatus::Code(_) | ExitStatus::Signal(_))
        ));

        state.shutdown().await;
    }
}
