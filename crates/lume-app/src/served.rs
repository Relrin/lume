//! A workspace whose layout a server owns.
//!
//! A backend whose server keeps the layout of its sessions gives a layout authority of
//! its own. The application then shows a copy of that layout in its windows: the
//! server's tabs and panes, with what is the application's own around them. What the
//! user asks to change in the tabs and panes is asked of the server, and takes effect
//! when the server says its layout changed; the copy is never changed ahead of it.

use std::sync::Arc;

use lume_config::schema::OnSwitch;
use lume_layout::{
    Applied, ChangeStream, Intent, LayoutAuthority, LayoutError, LayoutEvent, mirror,
};
use lume_proto::BackendId;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::error::CommandError;
use crate::panes::Panes;
use crate::tuning::{SERVED_COPIES_PER_WINDOW, SERVED_WINDOW, UNSOUND_NOTICE_EVERY};
use crate::workspaces::{self, Find};

/// The server whose layout is on show.
pub(crate) struct Served {
    pub(crate) backend: BackendId,
    pub(crate) authority: Arc<dyn LayoutAuthority>,
    /// The task that keeps the copy in step with the server.
    follower: JoinHandle<()>,
}

impl Drop for Served {
    fn drop(&mut self) {
        self.follower.abort();
    }
}

/// How many copies of a server's layout were made lately.
struct Pace {
    began: Instant,
    made: u32,
}

impl Pace {
    fn new() -> Self {
        Self {
            began: Instant::now(),
            made: 0,
        }
    }

    /// Counts one more copy, or says until when it has to wait.
    fn next(&mut self) -> Option<Instant> {
        if self.began.elapsed() >= SERVED_WINDOW {
            *self = Self::new();
        }
        if self.made >= SERVED_COPIES_PER_WINDOW {
            return Some(self.began + SERVED_WINDOW);
        }
        self.made += 1;
        None
    }
}

/// Takes the changes that have come already.
async fn passed_over(changes: &mut ChangeStream) {
    // Taking one that is there needs no waiting. What comes meanwhile is taken too, for
    // as long as a window is, and no longer.
    let until = Instant::now() + SERVED_WINDOW;
    while Instant::now() < until
        && matches!(
            tokio::time::timeout(std::time::Duration::ZERO, changes.next()).await,
            Ok(Some(_))
        )
    {}
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Panes {
    /// The backend whose server owns the layout on show, if a server does.
    pub fn served_by(&self) -> Option<BackendId> {
        lock(&self.served)
            .as_ref()
            .map(|served| served.backend.clone())
    }

    /// Whether the layout on show is the one this authority gives, and is followed.
    pub(crate) fn follows(
        &self,
        backend: &BackendId,
        authority: &Arc<dyn LayoutAuthority>,
    ) -> bool {
        lock(&self.served).as_ref().is_some_and(|served| {
            served.backend == *backend
                && std::ptr::addr_eq(Arc::as_ptr(&served.authority), Arc::as_ptr(authority))
                && !served.follower.is_finished()
        })
    }

    /// Shows the application's own workspace again that was last on show, when a
    /// server's layout is.
    pub async fn back_to_own(&self) -> Result<(), CommandError> {
        if self.served_by().is_none() {
            return Ok(());
        }
        let name = workspaces::start_workspace(
            None,
            workspaces::LAST,
            workspaces::last_workspace(self.config.dirs()),
        );
        self.switch(&name, Find::Either).await
    }

    /// The authority an intent is to be asked of, when that is not the application: a
    /// server owns the layout on show and the intent is not about what is the
    /// application's own.
    fn server_for(&self, intent: &Intent) -> Option<Arc<dyn LayoutAuthority>> {
        if intent.is_window_local() {
            return None;
        }
        lock(&self.served)
            .as_ref()
            .map(|served| Arc::clone(&served.authority))
    }

    /// Asks the server that owns the layout to apply an intent someone other than the
    /// application asked for. `None` when the intent is the application's to apply.
    pub(crate) async fn ask_server(
        &self,
        intent: &Intent,
    ) -> Option<Result<Applied, CommandError>> {
        let authority = self.server_for(intent)?;
        Some(self.asked(authority.apply(intent.clone()).await))
    }

    /// What whoever asked is told of a server's answer. The server numbers its own
    /// layout; whoever asked follows the copy, and is told where the copy will stand
    /// once it has taken the change: one step on from where it stands now.
    fn asked(&self, answer: Result<Applied, LayoutError>) -> Result<Applied, CommandError> {
        let applied = answer?;
        let shown = self.snapshot();
        Ok(Applied {
            epoch: shown.epoch,
            revision: shown.revision.saturating_add(1),
            created: applied.created,
        })
    }

    /// As [`ask_server`](Self::ask_server), for what the application itself asks. What
    /// only the application may apply to a layout of its own has no meaning in a
    /// server's, and is refused.
    pub(crate) async fn ask_server_as_core(
        &self,
        intent: &Intent,
    ) -> Option<Result<Applied, CommandError>> {
        let authority = self.server_for(intent)?;
        if intent.is_core_only() {
            return Some(Err(LayoutError::Unsupported(
                "the server that keeps this layout has no such thing to change".to_owned(),
            )
            .into()));
        }
        Some(self.asked(authority.apply(intent.clone()).await))
    }

    /// Refuses what can only be done to a workspace of the application's own.
    pub(crate) fn own_workspace(&self, what: &str) -> Result<(), CommandError> {
        match self.served_by() {
            Some(_) => Err(CommandError::new(
                "unsupported",
                format!("{what} is not possible while a server's layout is on show"),
            )),
            None => Ok(()),
        }
    }

    /// Stops following a server's layout, and says which server's. The copy stays on show
    /// until another workspace takes its place.
    pub(crate) fn leave_server(&self) -> Option<BackendId> {
        let left = lock(&self.served).take();
        left.map(|served| served.backend.clone())
    }

    /// Shows the layout that a backend's server keeps, in place of the workspace on
    /// show. The workspace that is put away is written first, and its programs end or
    /// carry on as the settings say for any workspace that is put away.
    pub async fn show_served(&self, backend: &BackendId) -> Result<(), CommandError> {
        let _one_at_a_time = self.switching.lock().await;
        let state = self
            .state
            .upgrade()
            .ok_or_else(|| CommandError::internal("the application is closing"))?;
        let authority = state.router().get(backend)?.layout().ok_or_else(|| {
            CommandError::new(
                "unsupported",
                format!("`{backend}` keeps no layout of its own"),
            )
        })?;
        // Taken before the layout is read, so no change falls between the two.
        let changes = authority.changes();
        let served = authority.view().await?;
        let old = self.workspace();
        let made = mirror(&old, served.workspace, backend).map_err(unsound)?;

        // What becomes of its programs is the say of the workspace being put away.
        let keep = self.config.current().config.workspace.on_switch == OnSwitch::Keep;
        let left = self.leave_server();
        // A copy of a server's layout is not written; a workspace of the application's is.
        self.save().await;
        // The settings of a workspace of this application do not apply to a server's.
        self.config.set_workspace(None);
        // The workspaces open beside the one put away are put away with it; they are
        // opened again when it is shown again.
        lock(&self.open).clear();
        self.layout.replace(made);
        if let Some(left) = left {
            self.forget_unshown(&left);
        }
        self.end_sessions_put_away(&old, keep);

        let this = state
            .panes()
            .ok_or_else(|| CommandError::internal("the application is closing"))?;
        let follower = state.runtime().spawn(Arc::clone(this).follow_server(
            backend.clone(),
            Arc::clone(&authority),
            changes,
        ));
        *lock(&self.served) = Some(Served {
            backend: backend.clone(),
            authority,
            follower,
        });
        self.sync_tab_panel().await;
        Ok(())
    }

    /// Keeps the copy on show in step with the server's layout until the server is
    /// gone or another workspace is shown.
    async fn follow_server(
        self: Arc<Self>,
        backend: BackendId,
        authority: Arc<dyn LayoutAuthority>,
        mut changes: ChangeStream,
    ) {
        // Said once in a while, not at every change that follows.
        let mut complained: Option<Instant> = None;
        let mut pace = Pace::new();
        while let Some(event) = changes.next().await {
            let mut reset = match event {
                LayoutEvent::Reset(snapshot) => Some(snapshot.workspace),
                LayoutEvent::Changed { .. } => None,
            };
            // Every copy goes to every page whole or in part. A server that changes its
            // layout without end does not fill them with it: what it passes through is
            // left out.
            while let Some(until) = pace.next() {
                tokio::time::sleep_until(until).await;
                passed_over(&mut changes).await;
                reset = None;
            }
            let served = match reset {
                Some(served) => served,
                // The changes are to the server's layout, not to the copy, which also
                // holds what is the application's own. The copy is made again from the
                // whole, and its followers are sent what differs.
                None => match authority.view().await {
                    Ok(snapshot) => snapshot.workspace,
                    Err(error) => {
                        tracing::debug!(%backend, %error, "a server's layout could not be read");
                        continue;
                    }
                },
            };
            let mut refused = None;
            self.layout.update(|shown| {
                // Another workspace has taken the copy's place meanwhile.
                if shown.backend.as_ref() != Some(&backend) {
                    return None;
                }
                match mirror(shown, served, &backend) {
                    Ok(made) => Some(made),
                    Err(violations) => {
                        refused = Some(violations);
                        None
                    }
                }
            });
            match refused {
                Some(violations) => {
                    tracing::warn!(%backend, ?violations, "a server sent a layout that is not sound");
                    let due = complained.is_none_or(|at| at.elapsed() >= UNSOUND_NOTICE_EVERY);
                    if due {
                        complained = Some(Instant::now());
                        self.notice(
                            "warning",
                            "The server sent a layout that cannot be shown. The layout stays as it was.",
                        );
                    }
                }
                None => self.forget_unshown(&backend),
            }
        }
        // The copy stays on show as it was. Whoever keeps the connection to the server
        // says why it ended, and what can be done.
        tracing::info!(%backend, "a server's layout is not followed any more");
    }
}

fn unsound(violations: Vec<lume_layout::Violation>) -> CommandError {
    tracing::warn!(?violations, "a server sent a layout that is not sound");
    CommandError::new(
        "protocol_error",
        "the server sent a layout that cannot be shown",
    )
}

#[cfg(test)]
mod tests;
