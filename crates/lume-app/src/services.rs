//! What the application is made of once it shows terminals, in one place: its state,
//! its panes, its configuration, its commands and its windows.
//!
//! The commands a page may call and the control endpoint both act through this, so a
//! command run from a key and the same command run from the command line take one
//! path.

use std::sync::Arc;
use std::time::Duration;

use lume_layout::{Intent, NewContent, PaneId, SpawnRequest, WindowId};
use lume_proto::app::{AppReplyBody, PageRequestBody};
use lume_session::Launcher;
use serde_json::Value;

use crate::builtins;
use crate::configsvc::ConfigService;
use crate::control::Shell;
use crate::error::CommandError;
use crate::panes::Panes;
use crate::registry::{Registry, Runs};
use crate::state::AppState;
use crate::tuning::PAGE_ANSWER_TIMEOUT;
use crate::window::label_of;
use crate::workspaces::Find;

/// Who asked for a command to be run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asker {
    /// The page of the window the command is to run in.
    Page,
    /// Something outside the window: the command line.
    Outside,
}

/// The parts of an application that shows terminals.
pub struct Services {
    pub state: Arc<AppState>,
    pub panes: Arc<Panes>,
    pub config: Arc<ConfigService>,
    pub registry: Arc<Registry>,
    pub shell: Arc<dyn Shell>,
}

/// The session of a pane that runs `exec`: a program and its arguments.
pub fn exec_request(exec: &[String], cwd: Option<String>) -> Option<SpawnRequest> {
    let (program, args) = exec.split_first()?;
    Some(SpawnRequest {
        launcher: Some(Launcher::Shell {
            program: program.clone(),
            args: args.to_vec(),
            login: false,
        }),
        cwd,
        ..SpawnRequest::default()
    })
}

impl Services {
    /// Does what starting the application again asks for: its window comes to the
    /// front, the workspace named is put on show, begun if there is none of that
    /// name, and the program named is run in a new tab. Returns the pane the program
    /// runs in. `page_wait` is how long a window that has only just been opened is
    /// given to load its page, which says how large the tab's pane will be.
    pub async fn launch(
        &self,
        workspace: Option<&str>,
        exec: &[String],
        cwd: Option<String>,
        page_wait: Duration,
    ) -> Result<Option<PaneId>, CommandError> {
        if let Some(name) = workspace {
            self.panes.switch(name, Find::Either).await?;
        }
        self.shell.activate();
        let Some(request) = exec_request(exec, cwd) else {
            return Ok(None);
        };
        let window = self.panes.focused_window().ok_or_else(|| {
            CommandError::new(
                lume_proto::app::codes::NO_WINDOW,
                "the application has no window to open a tab in",
            )
        })?;
        let label = label_of(window);
        self.state
            .soon(page_wait, |state| state.page_of(&label))
            .await;
        let applied = self
            .panes
            .apply(Intent::TabNew {
                window,
                content: NewContent::Terminal(request),
                index: None,
                activate: true,
                group: None,
            })
            .await?;
        Ok(applied.created.pane)
    }

    /// Runs a command in a window.
    ///
    /// A command that runs in the application is carried out here. One that runs in a
    /// page is passed to the window's page, unless it was the page that asked: a page
    /// runs its own commands itself, and one that asks the application to has made a
    /// mistake.
    pub async fn run_command(
        &self,
        window: WindowId,
        id: &str,
        args: &Value,
        asker: Asker,
    ) -> Result<(), CommandError> {
        let spec = self
            .registry
            .get(id)
            .ok_or_else(|| CommandError::not_found(format!("there is no command called `{id}`")))?;
        match (spec.runs, asker) {
            (Runs::Core, _) => {
                let context = builtins::Context {
                    panes: &self.panes,
                    config: &self.config,
                    shell: self.shell.as_ref(),
                    window,
                    asker,
                };
                builtins::run(&context, id, args).await
            }
            (Runs::Page, Asker::Page) => Err(CommandError::bad_request(format!(
                "`{id}` is carried out in the page"
            ))),
            (Runs::Page, Asker::Outside) => {
                let page = self.state.page_of(&label_of(window)).ok_or_else(|| {
                    CommandError::new(
                        lume_proto::app::codes::NO_WINDOW,
                        format!("window {window} has no page that could run the command"),
                    )
                })?;
                let request = PageRequestBody::Command {
                    id: id.to_owned(),
                    args: args.clone(),
                };
                match page.ask(request, PAGE_ANSWER_TIMEOUT).await? {
                    AppReplyBody::Ran { .. } => Ok(()),
                    _ => Err(CommandError::internal(
                        "the page gave an answer that does not fit a command",
                    )),
                }
            }
        }
    }
}
