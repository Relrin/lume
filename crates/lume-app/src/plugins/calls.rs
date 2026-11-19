//! The calls of a plugin's code that reach past its own things: the clipboard, the
//! terminals and their sessions, the workspace and its layout, what it puts to the
//! user, the permissions it holds and the services other plugins offer.
//!
//! Each call that needs a permission has passed the broker before it comes here; a
//! permission with a scope (a plugin's services) is matched against it here, and the
//! outcome written to the audit log.

use std::sync::Arc;

use lume_layout::{Dir, Intent, NewContent, PaneId, SpawnRequest, UNITS};
use lume_plugin::broker::grant_of;
use lume_plugin::permissions::{Kind, Prompt};
use lume_plugin_api::types::{
    Grant, NoticeLevel, PaneInfo, PromptWhen, ServicesCall, SessionEntry, SessionsSpawn,
    SpawnPlace, UiToast, WorkspaceInfo,
};
use lume_plugin_api::{ApiError, Call, ErrorCode};
use lume_proto::app::{AppReplyBody, PageRequestBody};
use lume_proto::{ErrorCode as SessionCode, Responder};
use lume_session::SessionError;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::asking::plain;
use super::{COMMAND_WAIT, Plugins, api_error};
use crate::carrier::SessionKey;
use crate::error::CommandError;
use crate::tuning::PAGE_ANSWER_TIMEOUT;
use crate::window::label_of;

/// The most a plugin puts on the clipboard, or types into a terminal at once.
const CLIPBOARD_MAX: usize = 1024 * 1024;
const TYPED_MAX: usize = 64 * 1024;

/// How long a plugin's notice may be.
const TOAST_MAX: usize = 300;

/// A failure of the application's own, as a plugin's code is told it.
fn from_command(error: CommandError) -> ApiError {
    let code = match error.code.as_ref() {
        "not_found" => ErrorCode::NotFound,
        "bad_request" => ErrorCode::Invalid,
        lume_proto::app::codes::TIMEOUT => ErrorCode::TimedOut,
        _ => ErrorCode::Failed,
    };
    api_error(code, error.message)
}

fn from_session(error: SessionError) -> ApiError {
    from_command(CommandError::from(error))
}

fn value_of<T: serde::Serialize>(value: &T) -> Result<Value, ApiError> {
    serde_json::to_value(value).map_err(|error| api_error(ErrorCode::Failed, error.to_string()))
}

impl Plugins {
    /// Carries out a call of the kinds this module holds; the rest go to the services
    /// that reach the machine.
    pub(super) async fn serve_more(
        self: &Arc<Self>,
        local: &str,
        call: Call,
    ) -> Result<Value, ApiError> {
        match call {
            Call::ClipboardRead(_) => {
                let text = self.clipboard()?.read().await.map_err(from_command)?;
                Ok(json!(text))
            }
            Call::ClipboardWrite(write) => {
                if write.text.len() > CLIPBOARD_MAX {
                    return Err(api_error(
                        ErrorCode::TooLarge,
                        "a plugin puts 1 MiB on the clipboard at most",
                    ));
                }
                self.clipboard()?
                    .write(write.text)
                    .await
                    .map_err(from_command)?;
                Ok(Value::Null)
            }
            Call::PanesList(_) => value_of(&self.pane_list()?),
            Call::PanesText(asked) => self.pane_text(&asked.pane).await.map(Value::String),
            Call::PanesWrite(write) => {
                self.pane_write(local, &write.pane, &write.text).await?;
                Ok(Value::Null)
            }
            Call::SessionsSpawn(asked) => self.spawn_session(asked).await,
            Call::SessionsList(_) => self.session_list().await,
            Call::WorkspaceCurrent(_) => value_of(&self.workspace_info()?),
            Call::WorkspaceRoot(_) => Ok(json!(self.config.current().workspace.root.clone())),
            Call::LayoutApply(apply) => {
                let intent: Intent = serde_json::from_value(apply.intent)
                    .map_err(|error| api_error(ErrorCode::Invalid, error.to_string()))?;
                let services = self.services_or_fail()?;
                let applied = services.panes.apply(intent).await.map_err(from_command)?;
                value_of(&applied)
            }
            Call::UiToast(toast) => {
                self.toast(local, toast)?;
                Ok(Value::Null)
            }
            Call::UiConfirm(asked) => self.confirm(local, asked).await.map(Value::Bool),
            Call::UiPick(asked) => self.pick(local, asked).await.map(|chosen| json!(chosen)),
            Call::PermissionsRequest(asked) => {
                self.request_permission(local, asked).await.map(Value::Bool)
            }
            Call::PermissionsList(_) => value_of(&self.grants_list(local)),
            Call::ServicesCall(asked) => self.call_service(local, asked).await,
            other => self.serve_system(local, other).await,
        }
    }

    fn services_or_fail(&self) -> Result<Arc<crate::services::Services>, ApiError> {
        self.services()
            .ok_or_else(|| api_error(ErrorCode::Failed, "the application is closing"))
    }

    fn clipboard(&self) -> Result<&crate::clipboard::ClipboardThread, ApiError> {
        self.clipboard
            .get()
            .ok_or_else(|| api_error(ErrorCode::Unavailable, "there is no clipboard here"))
    }

    /// A pane named by a plugin, and the session it shows.
    pub(super) fn pane_session(&self, pane: &str) -> Result<(PaneId, SessionKey), ApiError> {
        let services = self.services_or_fail()?;
        let id: PaneId = pane
            .parse()
            .map_err(|reason: &str| api_error(ErrorCode::Invalid, reason))?;
        if services.panes.workspace().pane(id).is_none() {
            return Err(api_error(
                ErrorCode::NotFound,
                format!("there is no pane {pane}"),
            ));
        }
        let key = services.panes.session_of(id).ok_or_else(|| {
            api_error(ErrorCode::NotFound, format!("pane {pane} shows no session"))
        })?;
        Ok((id, key))
    }

    fn pane_list(&self) -> Result<Vec<PaneInfo>, ApiError> {
        let services = self.services_or_fail()?;
        let panes = &services.panes;
        let workspace = panes.workspace();
        let focused: Vec<PaneId> = workspace
            .windows
            .iter()
            .filter_map(|window| panes.focused_pane(window.id))
            .collect();
        Ok(workspace
            .panes()
            .into_iter()
            .filter(|pane| panes.terminal(pane.id).is_some())
            .map(|pane| {
                let key = panes.session_of(pane.id);
                let reported = key
                    .as_ref()
                    .and_then(|key| self.state.reports().state(key))
                    .and_then(|state| state.title);
                PaneInfo {
                    pane: pane.id.to_string(),
                    session: key.map(|key| key.session.to_string()),
                    title: pane.title.clone().or(reported),
                    focused: focused.contains(&pane.id),
                }
            })
            .collect())
    }

    /// What a pane shows: from the session's host where it keeps the screen, or else
    /// from the page of the window that shows the pane.
    async fn pane_text(&self, pane: &str) -> Result<String, ApiError> {
        let (id, key) = self.pane_session(pane)?;
        let backend = self
            .state
            .router()
            .get(&key.backend)
            .map_err(from_session)?;
        if backend.caps().responder == Responder::Backend {
            let screen = backend.screen(key.session).await.map_err(from_session)?;
            return Ok(screen.lines.join("\n"));
        }
        let services = self.services_or_fail()?;
        let window = services.panes.workspace().window_of(id).ok_or_else(|| {
            api_error(ErrorCode::NotFound, format!("pane {pane} is in no window"))
        })?;
        let page = self.state.page_of(&label_of(window)).ok_or_else(|| {
            api_error(
                ErrorCode::Failed,
                format!("the window of pane {pane} shows nothing yet"),
            )
        })?;
        let screen = page
            .ask(
                PageRequestBody::Screen {
                    pane: Some(id.to_string()),
                    session: None,
                },
                PAGE_ANSWER_TIMEOUT,
            )
            .await
            .map_err(|error| api_error(ErrorCode::Failed, error.message))?;
        match screen {
            AppReplyBody::Screen { lines, .. } => Ok(lines.join("\n")),
            _ => Err(api_error(
                ErrorCode::Failed,
                "the window gave an answer that does not fit a screen",
            )),
        }
    }

    /// Types into a pane, once the user approves it where the permission says so.
    async fn pane_write(&self, local: &str, pane: &str, text: &str) -> Result<(), ApiError> {
        if text.len() > TYPED_MAX {
            return Err(api_error(
                ErrorCode::TooLarge,
                "a plugin types 64 KiB at most at once",
            ));
        }
        let (id, key) = self.pane_session(pane)?;
        let place = self
            .pane_list()?
            .into_iter()
            .find(|info| info.pane == id.to_string())
            .map(|info| match info.title {
                Some(title) => format!("pane {pane}, {title}"),
                None => format!("pane {pane}"),
            });
        self.approve_use(local, Kind::PanesWrite, text, place)
            .await?;
        let backend = self
            .state
            .router()
            .get(&key.backend)
            .map_err(from_session)?;
        match backend.send(key.session, text).await {
            Ok(()) => return Ok(()),
            Err(SessionError::Refused(error)) if error.code == SessionCode::Unsupported => {}
            Err(error) => return Err(from_session(error)),
        }
        let services = self.services_or_fail()?;
        let window = services.panes.workspace().window_of(id).ok_or_else(|| {
            api_error(ErrorCode::NotFound, format!("pane {pane} is in no window"))
        })?;
        let input = self
            .state
            .page_of(&label_of(window))
            .and_then(|page| page.input_of(&key))
            .ok_or_else(|| {
                api_error(
                    ErrorCode::Failed,
                    format!("pane {pane} is not shown yet, so it cannot be typed into"),
                )
            })?;
        input.write(text.as_bytes()).await.map_err(from_session)
    }

    async fn spawn_session(&self, asked: SessionsSpawn) -> Result<Value, ApiError> {
        let services = self.services_or_fail()?;
        let panes = &services.panes;
        let profile = match asked.profile.filter(|id| !id.trim().is_empty()) {
            Some(id) => match panes.profiles.resolve(&id).await {
                Some(_) => Some(id),
                None => {
                    return Err(api_error(
                        ErrorCode::NotFound,
                        format!("there is no profile called `{id}`"),
                    ));
                }
            },
            None => None,
        };
        let window = panes
            .focused_window()
            .ok_or_else(|| api_error(ErrorCode::Failed, "there is no window to open it in"))?;
        let content = NewContent::Terminal(SpawnRequest {
            profile,
            cwd: asked.options.cwd,
            ..SpawnRequest::default()
        });
        let place = asked.options.place.unwrap_or_default();
        let intent = match (place, panes.focused_pane(window)) {
            (SpawnPlace::Right | SpawnPlace::Down, Some(pane)) => Intent::Split {
                pane,
                dir: if place == SpawnPlace::Right {
                    Dir::Right
                } else {
                    Dir::Down
                },
                ratio: UNITS / 2,
                content,
            },
            _ => Intent::TabNew {
                window,
                content,
                index: None,
                activate: true,
                group: None,
            },
        };
        let applied = panes.apply(intent).await.map_err(from_command)?;
        let pane = applied
            .created
            .pane
            .ok_or_else(|| api_error(ErrorCode::Failed, "no pane was made"))?;
        let info = self
            .pane_list()?
            .into_iter()
            .find(|info| info.pane == pane.to_string())
            .unwrap_or(PaneInfo {
                pane: pane.to_string(),
                session: None,
                title: None,
                focused: true,
            });
        value_of(&info)
    }

    async fn session_list(&self) -> Result<Value, ApiError> {
        let services = self.services_or_fail()?;
        let listed = self.state.list_all().await.map_err(from_command)?;
        let entries: Vec<SessionEntry> = listed
            .into_iter()
            .map(|listed| {
                let key = SessionKey {
                    backend: listed.backend,
                    session: listed.info.id,
                };
                let state = self.state.reports().state(&key);
                SessionEntry {
                    session: listed.info.id.to_string(),
                    pane: services.panes.pane_of(&key).map(|pane| pane.to_string()),
                    program: listed.info.program,
                    title: listed
                        .info
                        .title
                        .or_else(|| state.as_ref().and_then(|state| state.title.clone())),
                    cwd: listed
                        .info
                        .cwd
                        .or_else(|| state.and_then(|state| state.cwd)),
                    exited: listed.info.exit.is_some(),
                }
            })
            .collect();
        value_of(&entries)
    }

    /// The workspace in use, as plugins hear of it.
    pub(super) fn workspace_info(&self) -> Result<WorkspaceInfo, ApiError> {
        let services = self.services_or_fail()?;
        let workspace = services.panes.workspace();
        let tabs = workspace
            .windows
            .iter()
            .map(|window| window.centre.tabs.len())
            .sum::<usize>();
        Ok(WorkspaceInfo {
            name: workspace.name.clone(),
            root: self.config.current().workspace.root.clone(),
            windows: u32::try_from(workspace.windows.len()).unwrap_or(u32::MAX),
            tabs: u32::try_from(tabs).unwrap_or(u32::MAX),
            panes: u32::try_from(workspace.panes().len()).unwrap_or(u32::MAX),
        })
    }

    fn toast(&self, local: &str, toast: UiToast) -> Result<(), ApiError> {
        if toast.message.chars().count() > TOAST_MAX {
            return Err(api_error(
                ErrorCode::Invalid,
                format!("a notice is {TOAST_MAX} characters at most"),
            ));
        }
        let name = self
            .inner()
            .loaded
            .get(local)
            .map(|loaded| loaded.installed.name())
            .unwrap_or_else(|| local.to_owned());
        let level = match toast.level.unwrap_or_default() {
            NoticeLevel::Info => "info",
            NoticeLevel::Warning => "warning",
            NoticeLevel::Error => "error",
        };
        let message = plain(&toast.message, TOAST_MAX).replace('\n', " ");
        self.notice(level, format!("{name}: \u{201c}{message}\u{201d}"));
        Ok(())
    }

    fn grants_list(&self, local: &str) -> Vec<Grant> {
        let inner = self.inner();
        let Some(loaded) = inner.loaded.get(local) else {
            return Vec::new();
        };
        loaded
            .grants
            .required
            .iter()
            .map(|grant| Grant {
                permission: grant.kind.as_str().to_owned(),
                scope: grant.scope.clone(),
                prompt: match grant.prompt {
                    Prompt::Once => PromptWhen::Once,
                    Prompt::Each => PromptWhen::Each,
                },
            })
            .collect()
    }

    /// Calls a service of the install the caller uses for the plugin it names: once its
    /// grant covers that plugin, the install offers the service, and the user agreed to
    /// the caller's using that install.
    async fn call_service(
        self: &Arc<Self>,
        local: &str,
        asked: ServicesCall,
    ) -> Result<Value, ApiError> {
        let (covered, bound, pinned) = {
            let inner = self.inner();
            let loaded = inner
                .loaded
                .get(local)
                .ok_or_else(|| api_error(ErrorCode::Failed, format!("{local} is not loaded")))?;
            let covered = grant_of(&loaded.grants, Kind::ServicesUse)
                .is_some_and(|grant| grant.scope.contains(&asked.plugin));
            let entry = &loaded.installed.entry;
            let bound = entry
                .bindings
                .get(&asked.plugin)
                .cloned()
                .unwrap_or_else(|| asked.plugin.clone());
            (covered, bound, entry.services.get(&asked.plugin).cloned())
        };
        if !covered {
            self.broker
                .record(local, Kind::ServicesUse, Some(&asked.plugin), false);
            return Err(api_error(
                ErrorCode::Denied,
                format!(
                    "{local} has no permission to use what {} offers",
                    asked.plugin
                ),
            ));
        }
        {
            let inner = self.inner();
            let offered = inner
                .loaded
                .get(&bound)
                .filter(|loaded| loaded.is_ready())
                .and_then(|loaded| loaded.manifest())
                .ok_or_else(|| {
                    api_error(
                        ErrorCode::NotFound,
                        format!("{bound} is not installed, or cannot be used"),
                    )
                })?
                .contributes
                .services
                .iter()
                .any(|service| service.name == asked.name);
            if !offered {
                return Err(api_error(
                    ErrorCode::NotFound,
                    format!("{bound} offers no service called {}", asked.name),
                ));
            }
        }
        if pinned.as_deref() != Some(bound.as_str()) {
            self.approve_binding(local, &asked.plugin, &bound).await?;
            if let Some(loaded) = self.inner().loaded.get_mut(local) {
                loaded
                    .installed
                    .entry
                    .services
                    .insert(asked.plugin.clone(), bound.clone());
            }
        } else {
            self.broker
                .record(local, Kind::ServicesUse, Some(&asked.plugin), true);
        }
        self.activate(&bound).await.map_err(|problem| {
            api_error(
                ErrorCode::Failed,
                format!("{bound} could not start: {problem}"),
            )
        })?;
        let (plugin, generation) = {
            let inner = self.inner();
            let running = inner
                .loaded
                .get(&bound)
                .and_then(|loaded| loaded.running.as_ref())
                .ok_or_else(|| api_error(ErrorCode::Stopped, format!("{bound} stopped")))?;
            (Arc::clone(&running.plugin), running.generation)
        };
        let invocation = self.next();
        let (sender, receiver) = oneshot::channel();
        self.inner()
            .invocations
            .insert(invocation, (bound.clone(), generation, sender));
        plugin.service(invocation, &asked.name, asked.args.unwrap_or(Value::Null));
        match tokio::time::timeout(COMMAND_WAIT, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(api_error(
                ErrorCode::Stopped,
                format!("{bound} stopped while it served {}", asked.name),
            )),
            Err(_) => {
                self.inner().invocations.remove(&invocation);
                Err(api_error(
                    ErrorCode::TimedOut,
                    format!(
                        "{bound} did not answer {} in {} seconds",
                        asked.name,
                        COMMAND_WAIT.as_secs()
                    ),
                ))
            }
        }
    }
}
