//! The calls of a plugin's code that reach past its own things: the clipboard, the
//! terminals and their sessions, the workspace and its layout, what it puts to the
//! user, the permissions it holds and the services other plugins offer.
//!
//! Each call that needs a permission has passed the broker before it comes here; a
//! permission with a scope (a plugin's services) is matched against it here, and the
//! outcome written to the audit log.

use std::sync::Arc;

use lume_layout::{Dir, Intent, NewContent, PaneContent, PaneId, SpawnRequest, UNITS};
use lume_plugin::broker::grant_of;
use lume_plugin::permissions::{Kind, Prompt};
use lume_plugin_api::types::{
    Grant, NoticeLevel, PaneInfo, PromptWhen, ServicesCall, SessionEntry, SessionsSpawn,
    SpawnPlace, UiToast, WorkspaceInfo,
};
use lume_plugin_api::{ApiError, Call, ErrorCode};
use lume_proto::app::{AppReplyBody, PageRequestBody};
use lume_proto::{ErrorCode as SessionCode, Launcher, Responder};
use lume_session::{SessionError, start_dir_from_report};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::asking::plain;
use super::{Plugins, api_error, command_wait};
use crate::carrier::SessionKey;
use crate::error::CommandError;
use crate::state::Listed;
use crate::tuning::PAGE_ANSWER_TIMEOUT;
use crate::window::label_of;

/// The most a plugin puts on the clipboard, or types into a terminal at once.
const CLIPBOARD_MAX: usize = 1024 * 1024;
const TYPED_MAX: usize = 64 * 1024;

/// How long a plugin's notice may be.
const TOAST_MAX: usize = 300;

/// What goes before the key of a plugin's picture where a pane names it as its own.
const PICTURE_IMAGE: &str = "image:";

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

/// A path without the long form's prefix (`\\?\C:\dir` as `C:\dir`), which the command
/// interpreter and many programs do not take.
fn plain_path(path: &str) -> String {
    match path.strip_prefix(r"\\?\") {
        Some(rest) => match rest.strip_prefix(r"UNC\") {
            Some(share) => format!(r"\\{share}"),
            None => rest.to_owned(),
        },
        None => path.to_owned(),
    }
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
            Call::SessionsSpawn(asked) => self.spawn_session(local, asked).await,
            Call::SessionsList(_) => self.session_list().await,
            Call::WorkspaceCurrent(_) => value_of(&self.workspace_info()?),
            Call::WorkspaceRoot(_) => Ok(json!(self.config.current().workspace.root.clone())),
            Call::LayoutApply(apply) => {
                let mut intent: Intent = serde_json::from_value(apply.intent)
                    .map_err(|error| api_error(ErrorCode::Invalid, error.to_string()))?;
                self.check_new_terminal(local, &mut intent).await?;
                self.own_picture(local, &mut intent)?;
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

    /// The picture a plugin gives a pane: one of the plugin's own, by the name its
    /// manifest gives it, or else one of the application's tiles. A plugin names no
    /// other plugin's picture.
    fn own_picture(&self, local: &str, intent: &mut Intent) -> Result<(), ApiError> {
        let Intent::PaneIcon {
            icon: Some(icon), ..
        } = intent
        else {
            return Ok(());
        };
        let own = format!("{local}:{icon}");
        if self.inner().shown.contributions.images.contains_key(&own) {
            *icon = format!("{PICTURE_IMAGE}{own}");
            return Ok(());
        }
        if icon.contains(':') {
            return Err(api_error(
                ErrorCode::Invalid,
                "a pane's picture is one of the plugin's own, or a tile, by its name",
            ));
        }
        Ok(())
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

    /// How a program a plugin names is started in a terminal: found along the path once
    /// and checked against the plugin's `process.run` grant, as a program it runs itself
    /// is. A batch file is run by the command interpreter, which the grant must hold.
    async fn terminal_program(
        &self,
        local: &str,
        name: &str,
        args: Vec<String>,
    ) -> Result<Launcher, ApiError> {
        let found = self.program(local, name).await?;
        let path = plain_path(&found.path.display().to_string());
        let batch = found
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
            });
        if !(cfg!(windows) && batch) {
            return Ok(Launcher::Shell {
                program: path,
                args,
                login: false,
            });
        }
        // The interpreter reads its line by rules of its own, under which a quoted
        // argument beside a quoted path, or one of its special characters, changes what
        // is run. Such arguments are refused instead of guessed at.
        let unsafe_for_batch = |text: &str| {
            text.is_empty()
                || text
                    .chars()
                    .any(|c| c.is_whitespace() || "\"&|<>^%!()".contains(c))
        };
        if args.iter().any(|arg| unsafe_for_batch(arg)) {
            return Err(api_error(
                ErrorCode::Invalid,
                format!(
                    "`{name}` is a batch file here, which cannot be given an argument that is \
                     empty or holds a space or one of \"&|<>^%!()"
                ),
            ));
        }
        let mut line = vec!["/d".to_owned(), "/c".to_owned(), path];
        line.extend(args);
        Ok(Launcher::Shell {
            program: "cmd.exe".to_owned(),
            args: line,
            login: false,
        })
    }

    /// Checks an intent of a plugin's that opens a terminal: opening one is starting a
    /// session, which needs `sessions.spawn`, and a program it names is checked as
    /// `terminal_program` checks it, and started from where it was found. Anything but a
    /// program by its name, or a session that already runs, is refused.
    async fn check_new_terminal(&self, local: &str, intent: &mut Intent) -> Result<(), ApiError> {
        let Some(NewContent::Terminal(request)) = intent.new_content_mut() else {
            return Ok(());
        };
        let may_spawn = self
            .inner()
            .loaded
            .get(local)
            .is_some_and(|loaded| grant_of(&loaded.grants, Kind::SessionsSpawn).is_some());
        self.broker
            .record(local, Kind::SessionsSpawn, None, may_spawn);
        if !may_spawn {
            return Err(api_error(
                ErrorCode::Denied,
                format!("{local} has no permission to start terminals"),
            ));
        }
        if request.session.is_some() || request.backend.is_some() {
            return Err(api_error(
                ErrorCode::Denied,
                "a plugin opens a new terminal on this computer, not a session that runs \
                 already or one elsewhere",
            ));
        }
        match request.launcher.take() {
            None => Ok(()),
            Some(Launcher::Shell { program, args, .. }) => {
                request.launcher = Some(self.terminal_program(local, &program, args).await?);
                Ok(())
            }
            Some(_) => Err(api_error(
                ErrorCode::Denied,
                "a plugin starts a program by its name, or a profile of the user's",
            )),
        }
    }

    async fn spawn_session(&self, local: &str, asked: SessionsSpawn) -> Result<Value, ApiError> {
        let services = self.services_or_fail()?;
        let panes = &services.panes;
        let options = asked.options;
        let launcher = match options.program.filter(|name| !name.trim().is_empty()) {
            Some(name) => Some(self.terminal_program(local, &name, options.args).await?),
            None if !options.args.is_empty() || options.env.is_some() => {
                return Err(api_error(
                    ErrorCode::Invalid,
                    "arguments and variables are for a program, which was not named",
                ));
            }
            None => None,
        };
        let beside = match options.pane {
            Some(pane) => {
                let id: PaneId = pane
                    .parse()
                    .map_err(|reason: &str| api_error(ErrorCode::Invalid, reason))?;
                if panes.workspace().pane(id).is_none() {
                    return Err(api_error(
                        ErrorCode::NotFound,
                        format!("there is no pane {pane}"),
                    ));
                }
                Some(id)
            }
            None => None,
        };
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
            launcher,
            cwd: options.cwd,
            env: options.env.unwrap_or_default().into_iter().collect(),
            ..SpawnRequest::default()
        });
        let place = options.place.unwrap_or_default();
        let intent = match (place, beside.or_else(|| panes.focused_pane(window))) {
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

    /// Every session, each with its folder where that is one of this machine's.
    pub(crate) async fn session_list(&self) -> Result<Value, ApiError> {
        let services = self.services_or_fail()?;
        let listed = self.state.list_all().await.map_err(from_command)?;
        let said: Vec<_> = listed.iter().map(|listed| self.said_dir(listed)).collect();
        let folders = tokio::task::spawn_blocking(move || {
            said.into_iter()
                .map(|said| {
                    let (launcher, url) = said?;
                    start_dir_from_report(&launcher, &url)
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|error| api_error(ErrorCode::Failed, error.to_string()))?;
        let entries: Vec<SessionEntry> = listed
            .into_iter()
            .zip(folders)
            .map(|(listed, cwd)| {
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
                        .or_else(|| state.and_then(|state| state.title)),
                    cwd,
                    exited: listed.info.exit.is_some(),
                }
            })
            .collect();
        value_of(&entries)
    }

    /// What a session said its folder is, with what it runs, for the folder to be looked
    /// for on this machine. Of a server's session only the server's own word counts,
    /// which it gives where it runs on this machine, and nothing that was printed.
    fn said_dir(&self, listed: &Listed) -> Option<(Launcher, String)> {
        if !self.state.is_local(&listed.backend) {
            let launcher = Launcher::Shell {
                program: String::new(),
                args: Vec::new(),
                login: false,
            };
            return Some((launcher, listed.info.cwd.clone()?));
        }
        let key = SessionKey {
            backend: listed.backend.clone(),
            session: listed.info.id,
        };
        let reports = self.state.reports();
        let launched = reports.launched(&key)?;
        let reported = || reports.state(&key).and_then(|state| state.cwd);
        let url = listed.info.cwd.clone().or_else(reported)?;
        Some((launched.launcher, url))
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

    /// Whether the plugin shows views in panes and every one of them is in a muted tab.
    fn muted_everywhere(&self, local: &str) -> bool {
        let Ok(services) = self.services_or_fail() else {
            return false;
        };
        let workspace = services.panes.workspace();
        let mut views = workspace
            .panes()
            .into_iter()
            .filter(|pane| {
                matches!(&pane.content, PaneContent::PluginView { plugin, .. } if plugin == local)
            })
            .peekable();
        views.peek().is_some() && views.all(|pane| workspace.pane_muted(pane.id))
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
        // A plugin whose views are all in tabs the user muted says nothing: its notice
        // is taken and not shown, as a bell in such a tab is not rung. One with a view
        // elsewhere, or with none, is heard.
        if self.muted_everywhere(local) {
            return Ok(());
        }
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
        match self.asked.within(command_wait(), receiver).await {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(api_error(
                ErrorCode::Stopped,
                format!("{bound} stopped while it served {}", asked.name),
            )),
            None => {
                self.inner().invocations.remove(&invocation);
                Err(api_error(
                    ErrorCode::TimedOut,
                    format!(
                        "{bound} did not answer {} in {} seconds",
                        asked.name,
                        command_wait().as_secs()
                    ),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use lume_layout::Store;
    use lume_proto::{BackendId, SessionEvent, SessionInfo};
    use lume_session::cwd::{file_url_of, machine_name};
    use lume_session::{HostConfig, LocalBackend, Router};
    use tokio::runtime::Handle;

    use super::*;
    use crate::configsvc::ConfigService;
    use crate::marks::Marks;
    use crate::panes::{PaneStart, Panes};
    use crate::services::Services;
    use crate::state::AppState;
    use crate::testing::{FakeServer, FakeShell, TempDir, server_session, size};
    use crate::tuning::WORKSPACE_NAME;

    /// The URL a shell of this machine reports when it is in `dir`.
    fn reported(dir: &Path) -> String {
        let url = file_url_of(&dir.to_string_lossy()).unwrap();
        url.replacen("file://", &format!("file://{}", machine_name().unwrap()), 1)
    }

    /// The application with a shell of its own running in a pane.
    async fn running(dir: &TempDir) -> (Arc<Services>, SessionKey) {
        std::fs::create_dir_all(dir.dirs().config).unwrap();
        let shell = if cfg!(windows) {
            "[terminal]\nshell = \"cmd.exe\"\nshell-args = [\"/d\"]\n"
        } else {
            "[terminal]\nshell = \"/bin/sh\"\n"
        };
        std::fs::write(dir.dirs().config_file(), shell).unwrap();
        let local = Arc::new(
            LocalBackend::in_process(HostConfig::default())
                .await
                .unwrap(),
        );
        let router = Router::new();
        router.insert(Router::local_id(), local.clone());
        let state = AppState::new(
            Handle::current(),
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
        let asked = vec![PaneStart {
            pane,
            size: size(80, 24),
        }];
        let started = panes.start_sessions(window, asked).await.started.remove(0);
        let registry = Arc::new(crate::builtins::registry());
        let plugins = Plugins::new(
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
            shell,
            plugins,
        });
        services.plugins.start(&services);
        let key = SessionKey {
            backend: started.backend,
            session: started.session,
        };
        (services, key)
    }

    fn says_it_is_in(services: &Services, key: &SessionKey, url: &str) {
        let event = SessionEvent::Cwd {
            url: url.to_owned(),
        };
        services.state.reports().on_event(key, &event);
    }

    /// The folder a plugin is given for a session.
    async fn folder(services: &Services, key: &SessionKey) -> Option<String> {
        let listed = services.plugins.session_list().await.unwrap();
        let listed: Vec<SessionEntry> = serde_json::from_value(listed).unwrap();
        listed
            .into_iter()
            .find(|entry| entry.session == key.session.to_string())
            .unwrap()
            .cwd
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_plugin_is_given_a_folder_only_where_it_is_one_of_this_machines() {
        let dir = TempDir::new("plugin-folders");
        let (services, key) = running(&dir).await;
        let here = dir.dirs().config;
        let found = Some(here.to_string_lossy().into_owned());
        assert_eq!(folder(&services, &key).await, None);
        says_it_is_in(&services, &key, &reported(&here));
        assert_eq!(folder(&services, &key).await, found);

        // Printed over a connection the user typed, and a folder that is not there.
        let machine = machine_name().unwrap();
        let other = reported(&here).replacen(&machine, "some-other-machine", 1);
        says_it_is_in(&services, &key, &other);
        assert_eq!(folder(&services, &key).await, None);
        says_it_is_in(&services, &key, &reported(&here.join("gone")));
        assert_eq!(folder(&services, &key).await, None);

        // A profile that connects to another machine, whatever that machine says.
        let remote = Launcher::Ssh {
            host: "box".to_owned(),
            user: None,
            port: None,
            extra_args: Vec::new(),
            client: None,
        };
        services.state.reports().started(key.clone(), remote, None);
        says_it_is_in(&services, &key, &reported(&here));
        assert_eq!(folder(&services, &key).await, None);
        services.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn of_a_servers_session_a_plugin_is_given_what_the_server_says_and_nothing_printed() {
        let dir = TempDir::new("plugin-server-folders");
        let (services, own) = running(&dir).await;
        let here = dir.dirs().config;
        let url = file_url_of(&here.to_string_lossy()).unwrap();
        let backend = BackendId("server".to_owned());
        let of = |raw: u64, cwd: Option<String>| SessionInfo {
            cwd,
            ..server_session(raw, "pwsh")
        };
        let sessions = vec![
            of(1, Some(url.clone())),
            of(2, None),
            of(3, Some(r"\\elsewhere\share".to_owned())),
            of(4, Some("file://elsewhere/share".to_owned())),
            of(5, Some(here.to_string_lossy().into_owned())),
        ];
        let keys: Vec<SessionKey> = sessions
            .iter()
            .map(|info| SessionKey {
                backend: backend.clone(),
                session: info.id,
            })
            .collect();
        services.state.router().insert(
            backend,
            Arc::new(FakeServer {
                sessions,
                ..FakeServer::default()
            }),
        );
        for key in &keys[1..] {
            says_it_is_in(&services, key, &reported(&here));
        }
        let given = Some(here.to_string_lossy().into_owned());
        assert_eq!(folder(&services, &keys[0]).await, given);
        for key in &keys[1..] {
            assert_eq!(folder(&services, key).await, None, "{key:?}");
        }
        assert_eq!(folder(&services, &own).await, None);
        services.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn what_a_servers_pane_uses_cannot_be_seen_and_no_other_server_is_asked() {
        let dir = TempDir::new("plugin-server-stats");
        let (services, own) = running(&dir).await;
        let panes = &services.panes;
        let backend = BackendId("server".to_owned());
        let session = server_session(1, "pwsh");
        let key = SessionKey {
            backend: backend.clone(),
            session: session.id,
        };
        let server = FakeServer {
            sessions: vec![session],
            ..FakeServer::default()
        };
        let other = Arc::new(FakeServer {
            silent: true,
            ..FakeServer::default()
        });
        let router = services.state.router();
        router.insert(backend, Arc::new(server));
        router.insert(BackendId("other".to_owned()), other.clone());
        let window = panes.focused_window().unwrap();
        crate::host::reattach(panes, window, key.clone(), crate::host::Place::Tab)
            .await
            .unwrap();
        let stats = async |key: &SessionKey| {
            let pane = panes.pane_of(key).unwrap().to_string();
            let asked = Call::PanesStats(lume_plugin_api::types::PaneRef { pane });
            services.plugins.serve_more("acme.metrics", asked).await
        };
        assert_eq!(stats(&key).await.unwrap_err().code, ErrorCode::Unavailable);
        assert!(stats(&own).await.unwrap()["processes"].as_u64() >= Some(1));
        assert_eq!(other.asked.load(std::sync::atomic::Ordering::Relaxed), 0);
        services.state.shutdown().await;
    }
}
