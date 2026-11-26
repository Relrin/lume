//! Carrying out a request: the one match over what the endpoint may be asked, each arm
//! passing the request on to the layout, the sessions, the pages or the plugins.

use super::*;

impl Control {
    pub(super) async fn carry_out(&self, body: AppRequestBody) -> Result<AppReply, AppError> {
        let Services {
            state,
            panes,
            registry,
            shell,
            ..
        } = self.services.as_ref();
        match body {
            AppRequestBody::Hello { .. } => Err(AppError::new(
                codes::BAD_REQUEST,
                "hello has been said on this connection",
            )),
            AppRequestBody::Launch {
                workspace,
                exec,
                cwd,
                rex,
            } => {
                if rex.is_some() && !exec.is_empty() {
                    return Err(AppError::new(
                        codes::BAD_REQUEST,
                        "a program is run or a Rex server is shown, not both",
                    ));
                }
                let pane = self
                    .services
                    .launch(workspace.as_deref(), &exec, cwd, self.page_wait)
                    .await?;
                let on_show = panes.workspace().name;
                let mut effect = "brought the window to the front".to_owned();
                if workspace.is_some() {
                    effect.push_str(&format!(", showing the workspace {on_show}"));
                }
                if let Some(pane) = pane {
                    effect.push_str(&format!(", and started the program in pane {pane}"));
                }
                if let Some((server, said)) = rex::start::wanted(rex.as_deref(), false, "") {
                    effect.push_str(&format!(", and shows the Rex server {server}"));
                    rex::start::begin(&self.services, server, said, self.page_wait);
                }
                reply(
                    effect,
                    AppReplyBody::Launched {
                        workspace: on_show,
                        pane: pane.map(|pane| pane.to_string()),
                    },
                )
            }
            AppRequestBody::Quit {} => reply("closing the application", AppReplyBody::Quitting {}),
            AppRequestBody::Status {} => {
                let focused = panes.focused_window().and_then(|window| {
                    let pane = panes.focused_pane(window)?;
                    Some(Focused {
                        pane: pane.to_string(),
                        session: panes.session_of(pane).map(|key| key.session),
                    })
                });
                reply(
                    "read the application's status",
                    AppReplyBody::Status {
                        app_version: env!("CARGO_PKG_VERSION").to_owned(),
                        pid: std::process::id(),
                        bundled_ui: shell.bundled_ui(),
                        window: state.page().is_some(),
                        focused,
                        marks: Box::new(state.marks().read()),
                    },
                )
            }
            AppRequestBody::Spawn { spec, place } => {
                let window = self.focused_window()?;
                // The page says how large the pane will be and shows what is started.
                let page = self.page_of(window).await?;
                let size = spec.size;
                // A profile that is not there is refused here, where the answer can
                // say so, rather than replaced by another when the pane starts.
                if let Some(id) = &spec.profile
                    && panes.profiles.resolve(id).await.is_none()
                {
                    return Err(AppError::new(
                        codes::NOT_FOUND,
                        format!("there is no profile called `{id}`"),
                    ));
                }
                let content = NewContent::Terminal(SpawnRequest {
                    backend: None,
                    profile: spec.profile,
                    launcher: spec.launcher,
                    cwd: spec.cwd,
                    env: spec.env,
                    size,
                    session: None,
                });
                let intent = match place {
                    SpawnPlace::Tab {} => Intent::TabNew {
                        window,
                        content,
                        index: None,
                        activate: true,
                        group: None,
                    },
                    SpawnPlace::Split { dir } => Intent::Split {
                        pane: self.place(None, None).await?.pane,
                        dir: layout_dir(dir),
                        ratio: UNITS / 2,
                        content,
                    },
                    SpawnPlace::Window {} => Intent::WindowNew {
                        content,
                        bounds: None,
                    },
                };
                let applied = panes.apply(intent).await?;
                let Some(pane) = applied.created.pane else {
                    return Err(AppError::new(
                        codes::INTERNAL,
                        "the intent was applied and made no pane",
                    ));
                };
                // The pane of a new window gets its session once the window's page has
                // loaded and said how large the pane is.
                let shown_in = applied.created.window.unwrap_or(window);
                let Some(key) = state
                    .soon(PAGE_OPEN_TIMEOUT, |_| panes.session_of(pane))
                    .await
                else {
                    return Err(AppError::new(
                        codes::TIMEOUT,
                        format!("pane {pane} was made, but no session was started in it in time"),
                    ));
                };
                // Typing into the session, or reading its screen, needs the page to
                // have attached to it. The answer waits for that, so a script's next
                // request finds the terminal ready.
                let label = label_of(shown_in);
                let page = if shown_in == window {
                    page
                } else {
                    self.page_of(shown_in).await?
                };
                let attached = state
                    .soon(PAGE_OPEN_TIMEOUT, |state| {
                        state.page_of(&label)?.input_of(&key)
                    })
                    .await;
                if attached.is_none() {
                    return Err(AppError::new(
                        codes::TIMEOUT,
                        format!(
                            "session {} was started in pane {pane}, but the window did not \
                             show it in time",
                            key.session
                        ),
                    ));
                }
                if let Some(size) = size {
                    // The caller chose the size: it holds, whatever fits the pane.
                    let pinned = PageRequestBody::Resize {
                        pane: Some(pane.to_string()),
                        session: None,
                        size,
                    };
                    page.ask(pinned, PAGE_ANSWER_TIMEOUT).await?;
                }
                let tab = applied.created.tab.map(|tab| tab.to_string());
                let effect = match (applied.created.window, &tab) {
                    (Some(opened), _) => format!(
                        "started session {} in pane {pane} of the new window {opened}",
                        key.session
                    ),
                    (None, Some(tab)) => format!(
                        "started session {} in pane {pane} of the new tab {tab}",
                        key.session
                    ),
                    (None, None) => {
                        format!("started session {} in the new pane {pane}", key.session)
                    }
                };
                reply(
                    effect,
                    AppReplyBody::Spawned {
                        session: key.session,
                        pane: pane.to_string(),
                        tab,
                    },
                )
            }
            AppRequestBody::List {} => {
                let sessions: Vec<AppSession> = state
                    .list_all()
                    .await?
                    .into_iter()
                    .map(|listed| {
                        let key = SessionKey {
                            backend: listed.backend.clone(),
                            session: listed.info.id,
                        };
                        AppSession {
                            pane: panes.pane_of(&key).map(|pane| pane.to_string()),
                            busy: state.reports().busy(&key, listed.info.exit.is_some()),
                            backend: listed.backend,
                            info: listed.info,
                        }
                    })
                    .collect();
                reply(
                    format!("listed {} sessions", sessions.len()),
                    AppReplyBody::Sessions { sessions },
                )
            }
            AppRequestBody::Kill { session } => {
                let key = state.find(session).await?;
                state.end(&key).await?;
                reply(format!("ended session {session}"), AppReplyBody::Killed {})
            }
            AppRequestBody::Send {
                pane,
                session,
                text,
            } => {
                let (key, place) = self.session(pane.as_deref(), session).await?;
                let typed = |key: &SessionKey| {
                    reply(
                        format!("typed {} bytes into session {}", text.len(), key.session),
                        AppReplyBody::Sent {
                            bytes: text.len() as u64,
                        },
                    )
                };
                // A backend that can be typed into directly needs no window.
                match self.backend_of(&key)?.send(key.session, &text).await {
                    Ok(()) => return typed(&key),
                    Err(SessionError::Refused(error)) if error.code == ErrorCode::Unsupported => {}
                    Err(error) => return Err(session_error(error)),
                }
                let not_shown = || {
                    AppError::new(
                        codes::NOT_SHOWN,
                        format!(
                            "session {} is not shown in a window, and only a shown session \
                             can be typed into",
                            key.session
                        ),
                    )
                };
                let label = label_of(place.ok_or_else(not_shown)?.window);
                // A session that has just been started is attached to a moment later.
                let input = state
                    .soon(self.page_wait, |state| {
                        state.page_of(&label)?.input_of(&key)
                    })
                    .await
                    .ok_or_else(not_shown)?;
                let bytes = text.into_bytes();
                if !bytes.is_empty() {
                    input
                        .write(&bytes)
                        .await
                        .map_err(|error| AppError::from(crate::error::CommandError::from(error)))?;
                }
                reply(
                    format!("typed {} bytes into session {}", bytes.len(), key.session),
                    AppReplyBody::Sent {
                        bytes: bytes.len() as u64,
                    },
                )
            }
            AppRequestBody::Resize {
                pane,
                session,
                size,
            } => {
                let (place, page) = self.screen_of(pane.as_deref(), session).await?;
                let resized = page
                    .ask(
                        PageRequestBody::Resize {
                            pane: Some(place.pane.to_string()),
                            session: None,
                            size,
                        },
                        PAGE_ANSWER_TIMEOUT,
                    )
                    .await?;
                match &resized {
                    AppReplyBody::Resized { size } => reply(
                        format!(
                            "the terminal in pane {} is now {} by {}",
                            place.pane,
                            size.cols(),
                            size.rows()
                        ),
                        resized,
                    ),
                    _ => Err(unexpected("resize")),
                }
            }
            AppRequestBody::WaitExit {
                pane,
                session,
                timeout_ms,
            } => {
                let (key, _) = self.session(pane.as_deref(), session).await?;
                let status = state.wait_exit(&key, wait_limit(timeout_ms)).await?;
                reply(
                    format!("session {} has ended", key.session),
                    AppReplyBody::Exited { status },
                )
            }
            AppRequestBody::Screen { pane, session } => {
                if let Some(answer) = self.host_screen(pane.as_deref(), session).await? {
                    return answer;
                }
                let (place, page) = self.screen_of(pane.as_deref(), session).await?;
                let screen = page
                    .ask(
                        PageRequestBody::Screen {
                            pane: Some(place.pane.to_string()),
                            session: None,
                        },
                        PAGE_ANSWER_TIMEOUT,
                    )
                    .await?;
                match &screen {
                    AppReplyBody::Screen { lines, .. } => reply(
                        format!(
                            "read {} lines of the screen of pane {}",
                            lines.len(),
                            place.pane
                        ),
                        screen,
                    ),
                    _ => Err(unexpected("screen")),
                }
            }
            AppRequestBody::WaitText {
                pane,
                session,
                text,
                timeout_ms,
            } => {
                let limit = wait_limit(timeout_ms);
                if let Some(answer) = self
                    .host_wait_text(pane.as_deref(), session, &text, limit)
                    .await?
                {
                    return answer;
                }
                let (place, page) = self.screen_of(pane.as_deref(), session).await?;
                // The page keeps the time. It is asked for a little longer, so that its
                // own answer that the time ran out arrives before this gives up on it.
                let found = page
                    .ask(
                        PageRequestBody::WaitText {
                            pane: Some(place.pane.to_string()),
                            session: None,
                            text,
                            timeout_ms: u64::try_from(limit.as_millis()).unwrap_or(u64::MAX),
                        },
                        limit + WAIT_SLACK,
                    )
                    .await?;
                match &found {
                    AppReplyBody::Matched { row, .. } => {
                        reply(format!("the text is on row {row}"), found)
                    }
                    _ => Err(unexpected("wait_text")),
                }
            }
            AppRequestBody::Layout {} => {
                let snapshot = panes.snapshot();
                let workspace = serde_json::to_value(&snapshot.workspace)
                    .map_err(|error| AppError::new(codes::INTERNAL, error.to_string()))?;
                reply(
                    format!("read the layout at revision {}", snapshot.revision),
                    AppReplyBody::Layout {
                        window: panes.focused_window().map(|window| window.to_string()),
                        epoch: snapshot.epoch,
                        revision: snapshot.revision,
                        workspace,
                    },
                )
            }
            AppRequestBody::LayoutApply { intent } => {
                let intent: Intent = serde_json::from_value(intent).map_err(|error| {
                    AppError::new(codes::BAD_REQUEST, format!("not an intent: {error}"))
                })?;
                let before = panes.snapshot().revision;
                let applied = panes.apply(intent).await?;
                let created = serde_json::to_value(applied.created)
                    .map_err(|error| AppError::new(codes::INTERNAL, error.to_string()))?;
                let effect = if applied.revision == before {
                    "the layout is as it was: the intent changed nothing".to_owned()
                } else {
                    format!("the layout is now at revision {}", applied.revision)
                };
                reply(
                    effect,
                    AppReplyBody::Applied {
                        epoch: applied.epoch,
                        revision: applied.revision,
                        created,
                    },
                )
            }
            AppRequestBody::Workspaces {} => {
                let (active, workspaces) = panes.workspaces()?;
                reply(
                    format!("listed {} workspaces", workspaces.len()),
                    AppReplyBody::Workspaces { active, workspaces },
                )
            }
            AppRequestBody::Profiles {} => {
                let mut profiles = panes.profiles.view();
                // The looks are for drawing a terminal, which the command line does not.
                if let Some(view) = profiles.as_object_mut() {
                    view.remove("looks");
                }
                let count = profiles["profiles"].as_array().map_or(0, Vec::len);
                reply(
                    format!("listed {count} profiles"),
                    AppReplyBody::Profiles { profiles },
                )
            }
            AppRequestBody::Rex {} => {
                let rex = self.services.rex.look().await;
                let count = rex["hosts"].as_array().map_or(0, Vec::len);
                reply(
                    format!("listed {count} Rex servers"),
                    AppReplyBody::Rex { rex },
                )
            }
            // Answered before it gets here, since its answer is not the end of it.
            AppRequestBody::Events { .. }
            | AppRequestBody::Attach { .. }
            | AppRequestBody::Verb { .. } => Err(AppError::new(
                codes::INTERNAL,
                "a request that goes on after its answer was handled as a plain request",
            )),
            // Answered before it gets here, for as long as its client is there.
            AppRequestBody::Askpass { .. } => Err(AppError::new(
                codes::INTERNAL,
                "a question of the SSH client's was handled as a plain request",
            )),
            AppRequestBody::Commands {} => {
                let commands = registry.list();
                let count = commands.len();
                let commands = serde_json::to_value(commands)
                    .map_err(|error| AppError::new(codes::INTERNAL, error.to_string()))?;
                reply(
                    format!("listed {count} commands"),
                    AppReplyBody::Commands { commands },
                )
            }
            AppRequestBody::Command { id, args, window } => {
                let window = match window {
                    Some(text) => {
                        let window: WindowId = text
                            .parse()
                            .map_err(|reason: &str| AppError::new(codes::BAD_REQUEST, reason))?;
                        if panes.workspace().window(window).is_none() {
                            return Err(AppError::new(
                                codes::NOT_FOUND,
                                format!("there is no window {window}"),
                            ));
                        }
                        window
                    }
                    None => self.focused_window()?,
                };
                // A command that runs in a page needs the page; one that starts a
                // session wants it too, to say how large the pane will be.
                let _ = self.page_of(window).await;
                self.services
                    .run_command(window, &id, &args, Asker::Outside)
                    .await?;
                reply(
                    format!("ran {id} in window {window}"),
                    AppReplyBody::Ran { command: id },
                )
            }
            AppRequestBody::Verbs {} => {
                let verbs = self.services.plugins.verbs();
                let count = verbs.len();
                let verbs = serde_json::to_value(verbs)
                    .map_err(|error| AppError::new(codes::INTERNAL, error.to_string()))?;
                reply(
                    format!("listed {count} verbs"),
                    AppReplyBody::Verbs { verbs },
                )
            }
            AppRequestBody::Plugins {} => {
                let plugins = self.services.plugins.describe();
                let count = plugins.as_array().map_or(0, Vec::len);
                reply(
                    format!("listed {count} plugins"),
                    AppReplyBody::Plugins { plugins },
                )
            }
            AppRequestBody::PluginActivate { id } => {
                self.services.plugins.activate_for_control(&id).await?;
                reply(
                    format!("{id} is running"),
                    AppReplyBody::PluginActivated { id },
                )
            }
        }
    }
}
