//! The plugins' views that are open in the windows, and the way between each one and its
//! plugin's code.
//!
//! A page shows a plugin's panel or pane view in a sandboxed frame and tells the
//! application when one opens and closes. Each open view gets an instance, which names
//! it to the plugin's code: calls the frame makes reach the code with it, and the code
//! sends events to one instance or to every open one of a view. Which plugin a frame
//! belongs to is what the page said when it opened it, from the application's own list
//! of views; a frame never names itself.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use lume_layout::{
    Area, Dir, FocusTarget, Intent, NewContent, PaneContent, PaneId, PanelContent, PanelId, UNITS,
    WindowId,
};
use lume_plugin_api::types::{
    OpenViewOptions, OpenedView, StreamGrant, ViewInstance, ViewPlace, ViewsPost,
};
use lume_plugin_api::{ApiError, ErrorCode};
use lume_services::streams::Streams;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::{COMMAND_WAIT, Plugins, api_error};
use crate::carrier::Pipe;
use crate::error::CommandError;
use crate::window::window_of;

/// The most a view's call or an event to a view may carry, as JSON.
pub const PAYLOAD_MAX: usize = 1024 * 1024;

/// The most views a window may have open at once.
const OPEN_MAX: usize = 256;

/// The longest name a call or an event may have.
const NAME_MAX: usize = 128;

/// The most a view may have kept for it, as JSON.
pub const STATE_MAX: usize = 64 * 1024;

/// How many grants to streams one open view may hold at once.
const GRANTS_PER_VIEW: usize = 16;

/// One open view.
#[derive(Clone, Debug)]
struct Open {
    local: String,
    view: String,
    place: Place,
    /// The pane or the panel the page says it shows the view in, by its id.
    host: Option<String>,
}

/// A view's leave to use one of its plugin's streams.
struct Granted {
    local: String,
    window: String,
    page: String,
    stream: u64,
    /// The carrier and the stream on it the view opened the grant as, once it has.
    bound: Option<(u32, u32)>,
}

/// One of a plugin's streams, as a link carries it for a view.
struct ViewPipe {
    streams: Arc<Streams>,
    owner: String,
    id: u64,
}

impl Pipe for ViewPipe {
    fn ways(&self) -> (bool, bool) {
        self.streams
            .ways(&self.owner, self.id)
            .unwrap_or((false, false))
    }

    fn read(&self, max: usize) -> Result<Option<Vec<u8>>, String> {
        self.streams
            .read(&self.owner, self.id, max)
            .map_err(|error| error.message)
    }

    fn write(&self, bytes: &[u8]) -> Result<(), String> {
        self.streams
            .write(&self.owner, self.id, bytes)
            .map_err(|error| error.message)
    }

    fn finish(&self) {
        let _ = self.streams.close(&self.owner, self.id);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Pane,
    Panel,
}

impl Place {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pane" => Some(Self::Pane),
            "panel" => Some(Self::Panel),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Pane => "pane",
            Self::Panel => "panel",
        }
    }
}

/// The open views, by window and the instance the page gave each.
#[derive(Default)]
pub struct Views {
    open: Mutex<HashMap<(String, String), Open>>,
    /// The grants to streams that views hold, by the grant.
    grants: Mutex<HashMap<String, Granted>>,
}

impl Views {
    fn open(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), Open>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn grants(&self) -> std::sync::MutexGuard<'_, HashMap<String, Granted>> {
        self.grants.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A grant nobody can guess: 16 bytes from the system, in hex.
fn new_grant() -> Result<String, ApiError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        api_error(
            ErrorCode::Failed,
            format!("no grant could be made: {error}"),
        )
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn too_large_state() -> CommandError {
    CommandError::bad_request("a view keeps 64 KiB at most")
}

fn state_fits(state: &Value) -> bool {
    fits(state, STATE_MAX)
}

/// Whether a value is `most` bytes or fewer as JSON. The value is measured as it would
/// be written, without writing it anywhere, and no further than the limit.
fn fits(value: &Value, most: usize) -> bool {
    struct Counted {
        left: usize,
    }
    impl std::io::Write for Counted {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.left = self
                .left
                .checked_sub(bytes.len())
                .ok_or_else(|| std::io::Error::other("past the limit"))?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counted { left: most }, value).is_ok()
}

/// The instance the plugin's code knows an open view by: the window's label and the
/// page's own name for it.
fn instance_of(window: &str, page: &str) -> String {
    format!("{window}/{page}")
}

fn described(window: &str, page: &str, open: &Open) -> ViewInstance {
    ViewInstance {
        instance: instance_of(window, page),
        view: open.view.clone(),
        place: open.place.name().to_owned(),
        pane: open.host.clone().filter(|_| open.place == Place::Pane),
    }
}

/// Whether a page's name for one of its views is one it may give: short, plain.
fn page_name_ok(page: &str) -> bool {
    !page.is_empty()
        && page.len() <= 64
        && page
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A grant, as whoever takes grants back picks among them.
pub(super) struct GrantRef<'a> {
    pub key: &'a str,
    pub local: &'a str,
    pub window: &'a str,
    pub page: &'a str,
}

impl Plugins {
    /// A page opened one of a plugin's views: `plugin` is the install, `view` the name
    /// its manifest gives it. Gives the instance the plugin's code knows it by.
    pub fn view_opened(
        self: &Arc<Self>,
        window: &str,
        page: &str,
        local: &str,
        view: &str,
        place: Place,
        host: Option<String>,
    ) -> Result<String, CommandError> {
        if !page_name_ok(page) || host.as_deref().is_some_and(|host| host.len() > 128) {
            return Err(CommandError::bad_request(
                "a view's name in its page is plain and short",
            ));
        }
        let known = {
            let inner = self.inner();
            let shown = &inner.shown.contributions;
            match place {
                Place::Pane => shown
                    .pane_views
                    .iter()
                    .any(|def| def.plugin == local && def.view == view),
                Place::Panel => shown
                    .panels
                    .iter()
                    .any(|def| def.plugin == local && def.view == view && def.entry.is_some()),
            }
        };
        if !known {
            return Err(CommandError::not_found(format!(
                "{local} has no {} called {view} that shows a page",
                place.name()
            )));
        }
        let open = Open {
            local: local.to_owned(),
            view: view.to_owned(),
            place,
            host,
        };
        let described = described(window, page, &open);
        {
            let mut views = self.views.open();
            let in_window = views.keys().filter(|(label, _)| label == window).count();
            let key = (window.to_owned(), page.to_owned());
            if !views.contains_key(&key) && in_window >= OPEN_MAX {
                return Err(CommandError::bad_request(
                    "too many views are open in the window",
                ));
            }
            views.insert(key, open);
        }
        self.tell_views(local, "open", &described);
        // A plugin that waits for its view is started by it.
        let starts = self.inner().loaded.get(local).is_some_and(|loaded| {
            loaded.manifest().is_some_and(|manifest| {
                manifest.events.iter().any(|event| {
                    matches!(event, lume_plugin::manifest::Activation::View(name) if name == view)
                })
            })
        });
        if starts {
            let this = Arc::clone(self);
            let local = local.to_owned();
            self.state.runtime().spawn(async move {
                if let Err(problem) = this.activate(&local).await {
                    tracing::warn!(plugin = %local, problem, "a plugin did not start for its view");
                }
            });
        }
        Ok(described.instance)
    }

    /// A page closed one of its views.
    pub fn view_closed(&self, window: &str, page: &str) {
        let gone = self
            .views
            .open()
            .remove(&(window.to_owned(), page.to_owned()));
        if let Some(open) = gone {
            self.drop_grants(|granted| granted.window == window && granted.page == page);
            self.tell_views(&open.local, "close", &described(window, page, &open));
        }
    }

    /// Keeps what one of the page's open views wants to find again, with the pane or
    /// the panel it is shown in: it is saved with the workspace, and the view is given
    /// it when it next starts. A pane view whose manifest says not to keeps nothing.
    pub async fn view_state(
        &self,
        window: &str,
        page: &str,
        state: Value,
    ) -> Result<(), CommandError> {
        if !state_fits(&state) {
            return Err(too_large_state());
        }
        let open = self
            .views
            .open()
            .get(&(window.to_owned(), page.to_owned()))
            .cloned()
            .ok_or_else(|| CommandError::not_found("the view is not open"))?;
        let Some(host) = open.host else {
            return Ok(());
        };
        let services = self
            .services()
            .ok_or_else(|| CommandError::internal("the application is closing"))?;
        let workspace = services.panes.workspace();
        let window = window_of(window)
            .ok_or_else(|| CommandError::forbidden("this window shows no part of the layout"))?;
        let intent = match open.place {
            Place::Pane => {
                let persists = self
                    .inner()
                    .shown
                    .contributions
                    .pane_views
                    .iter()
                    .any(|def| def.plugin == open.local && def.view == open.view && def.persist);
                if !persists {
                    return Ok(());
                }
                let pane: PaneId = host
                    .parse()
                    .map_err(|reason: &str| CommandError::bad_request(reason))?;
                // The page says where its view is; the layout says whether that is so.
                let shows = workspace.window_of(pane) == Some(window)
                    && workspace.pane(pane).is_some_and(|found| {
                        matches!(
                            &found.content,
                            PaneContent::PluginView { plugin, view, .. }
                                if *plugin == open.local && *view == open.view
                        )
                    });
                if !shows {
                    return Err(CommandError::not_found("the pane does not show that view"));
                }
                Intent::ViewState { pane, state }
            }
            Place::Panel => {
                let panel = PanelId::new(&host).map_err(|why| {
                    CommandError::bad_request(format!("`{host}` is not a panel id: {why}"))
                })?;
                let shows = workspace.window(window).is_some_and(|found| {
                    [Area::Left, Area::Right, Area::Bottom]
                        .into_iter()
                        .flat_map(|area| found.dock.area(area).panels.iter())
                        .any(|found| {
                            found.id == panel
                                && matches!(
                                    &found.content,
                                    PanelContent::PluginView { plugin, view, .. }
                                        if *plugin == open.local && *view == open.view
                                )
                        })
                });
                if !shows {
                    return Err(CommandError::not_found("the panel does not show that view"));
                }
                Intent::PanelState {
                    window,
                    panel,
                    state,
                }
            }
        };
        services.panes.apply_core(intent).await.map(drop)
    }

    /// Opens one of a plugin's pane views in a window, or shows a pane that has it
    /// open already where that is asked for.
    pub async fn open_view(
        &self,
        window: Option<WindowId>,
        local: &str,
        view: &str,
        options: OpenViewOptions,
    ) -> Result<OpenedView, CommandError> {
        let title = {
            let inner = self.inner();
            inner
                .shown
                .contributions
                .pane_views
                .iter()
                .find(|def| def.plugin == local && def.view == view)
                .map(|def| def.title.clone())
                .ok_or_else(|| {
                    CommandError::not_found(format!("{local} has no pane view called {view}"))
                })?
        };
        let state = options.state.unwrap_or(Value::Null);
        if !state_fits(&state) {
            return Err(too_large_state());
        }
        let services = self
            .services()
            .ok_or_else(|| CommandError::internal("the application is closing"))?;
        let panes = &services.panes;
        if options.reuse {
            let workspace = panes.workspace();
            let shown = workspace.panes().into_iter().find(|pane| {
                matches!(
                    &pane.content,
                    PaneContent::PluginView { plugin, view: shown, .. }
                        if plugin == local && shown == view
                ) && workspace.window_of(pane.id).is_some()
            });
            if let Some(pane) = shown {
                panes
                    .apply(Intent::Focus {
                        target: FocusTarget::Pane { pane: pane.id },
                    })
                    .await?;
                return Ok(OpenedView {
                    pane: pane.id.to_string(),
                    reused: true,
                });
            }
        }
        let window = window
            .or_else(|| panes.focused_window())
            .or_else(|| panes.workspace().windows.first().map(|window| window.id))
            .ok_or_else(|| CommandError::internal("there is no window to open it in"))?;
        let content = NewContent::PluginView {
            plugin: local.to_owned(),
            view: view.to_owned(),
            state,
        };
        let place = options.place.unwrap_or_default();
        let intent = match (place, panes.focused_pane(window)) {
            (ViewPlace::Right | ViewPlace::Down | ViewPlace::Float, Some(pane)) => Intent::Split {
                pane,
                dir: if place == ViewPlace::Down {
                    Dir::Down
                } else {
                    Dir::Right
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
        let applied = panes.apply(intent).await?;
        let pane = applied
            .created
            .pane
            .ok_or_else(|| CommandError::internal("no pane was made"))?;
        if place == ViewPlace::Float {
            panes.apply(Intent::Float { pane, rect: None }).await?;
        }
        let title = options
            .title
            .map(|title| super::asking::plain(title.trim(), 80).replace('\n', " "))
            .filter(|title| !title.is_empty())
            .unwrap_or(title);
        panes
            .apply(Intent::PaneRename {
                pane,
                title: Some(title),
            })
            .await?;
        Ok(OpenedView {
            pane: pane.to_string(),
            reused: false,
        })
    }

    /// `streams.grantToView`: lets one of the plugin's open views use one of its
    /// streams, and gives the grant the view opens it with.
    pub(super) fn stream_grant(&self, local: &str, asked: &StreamGrant) -> Result<Value, ApiError> {
        let (window, page) = self
            .views
            .open()
            .iter()
            .find(|((window, page), open)| {
                open.local == local && instance_of(window, page) == asked.instance
            })
            .map(|(key, _)| key.clone())
            .ok_or_else(|| {
                api_error(
                    ErrorCode::NotFound,
                    format!("the plugin has no open view {}", asked.instance),
                )
            })?;
        if self.system.streams().ways(local, asked.stream).is_none() {
            return Err(api_error(
                ErrorCode::NotFound,
                format!("there is no stream {}", asked.stream),
            ));
        }
        let grant = new_grant()?;
        let mut grants = self.views.grants();
        let held = grants
            .values()
            .filter(|granted| granted.window == window && granted.page == page)
            .count();
        if held >= GRANTS_PER_VIEW {
            return Err(api_error(
                ErrorCode::TooLarge,
                format!("a view holds {GRANTS_PER_VIEW} grants at most at once"),
            ));
        }
        grants.insert(
            grant.clone(),
            Granted {
                local: local.to_owned(),
                window,
                page,
                stream: asked.stream,
                bound: None,
            },
        );
        Ok(Value::String(grant))
    }

    /// `streams.revoke`: takes a grant of the plugin's back.
    pub(super) fn stream_revoke(&self, local: &str, grant: &str) {
        self.drop_grants(|granted| granted.local == local && granted.key == grant);
    }

    /// Takes back the grants `which` picks: a view that had opened one is told its
    /// stream is over.
    pub(super) fn drop_grants(&self, which: impl Fn(&GrantRef<'_>) -> bool) {
        let gone: Vec<Granted> = {
            let mut grants = self.views.grants();
            let keys: Vec<String> = grants
                .iter()
                .filter(|(key, granted)| {
                    which(&GrantRef {
                        key,
                        local: &granted.local,
                        window: &granted.window,
                        page: &granted.page,
                    })
                })
                .map(|(key, _)| key.clone())
                .collect();
            keys.into_iter()
                .filter_map(|key| grants.remove(&key))
                .collect()
        };
        for granted in gone {
            let Some((carrier, stream)) = granted.bound else {
                continue;
            };
            if let Some(link) = self
                .state
                .page_of(&granted.window)
                .filter(|link| link.id() == carrier)
            {
                link.end_pipe(stream);
            }
        }
    }

    /// One of the page's open views opens a grant its plugin's code handed it, as the
    /// stream `stream` of the page's carrier. A grant made for another view, even of
    /// the same plugin, is no grant here, and neither is one that was opened before.
    pub fn view_stream_open(
        self: &Arc<Self>,
        window: &str,
        carrier: u32,
        page: &str,
        grant: &str,
        stream: u32,
    ) -> Result<Value, CommandError> {
        let no_grant = || CommandError::not_found("there is no such grant");
        let link = self
            .state
            .page_of(window)
            .filter(|link| link.id() == carrier)
            .ok_or_else(CommandError::stale_carrier)?;
        let (local, id) = {
            let mut grants = self.views.grants();
            let granted = grants
                .get_mut(grant)
                .filter(|granted| {
                    granted.window == window && granted.page == page && granted.bound.is_none()
                })
                .ok_or_else(no_grant)?;
            link.reserve(stream)?;
            granted.bound = Some((carrier, stream));
            (granted.local.clone(), granted.stream)
        };
        let pipe = Arc::new(ViewPipe {
            streams: Arc::clone(self.system.streams()),
            owner: local,
            id,
        });
        let (readable, writable) = pipe.ways();
        let this = Arc::downgrade(self);
        let key = grant.to_owned();
        let bound = link.bind_pipe(
            stream,
            pipe,
            Box::new(move || {
                if let Some(this) = this.upgrade() {
                    this.views.grants().remove(&key);
                }
            }),
        );
        if let Err(error) = bound {
            self.views.grants().remove(grant);
            return Err(error);
        }
        Ok(json!({ "readable": readable, "writable": writable }))
    }

    /// A window went: its views went with it.
    pub fn views_of_window_gone(&self, window: &str) {
        let gone: Vec<(String, Open)> = {
            let mut views = self.views.open();
            let keys: Vec<(String, String)> = views
                .keys()
                .filter(|(label, _)| label == window)
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|key| views.remove(&key).map(|open| (key.1, open)))
                .collect()
        };
        self.drop_grants(|granted| granted.window == window);
        for (page, open) in gone {
            self.tell_views(&open.local, "close", &described(window, &page, &open));
        }
    }

    /// Tells a running plugin a view of its opened or closed.
    fn tell_views(&self, local: &str, kind: &str, view: &ViewInstance) {
        let plugin = self
            .inner()
            .loaded
            .get(local)
            .and_then(|loaded| loaded.running.as_ref())
            .filter(|running| running.started)
            .map(|running| Arc::clone(&running.plugin));
        if let Some(plugin) = plugin {
            plugin.event("views", json!({ "kind": kind, "view": view }));
        }
    }

    /// A plugin's code has started: it is told of its views that are open already.
    pub(super) fn tell_open_views(&self, local: &str) {
        let open: Vec<ViewInstance> = self
            .views
            .open()
            .iter()
            .filter(|(_, open)| open.local == local)
            .map(|((window, page), open)| described(window, page, open))
            .collect();
        for view in open {
            self.tell_views(local, "open", &view);
        }
    }

    /// A call an open view made with `host.call(name, args)`, passed to its plugin's
    /// code (started for it if need be). Answers what the code answers.
    pub async fn view_call(
        self: &Arc<Self>,
        window: &str,
        page: &str,
        name: &str,
        args: Value,
    ) -> Result<Value, ApiError> {
        if name.is_empty() || name.len() > NAME_MAX {
            return Err(api_error(
                ErrorCode::Invalid,
                "a call's name is 1 to 128 characters",
            ));
        }
        if !fits(&args, PAYLOAD_MAX) {
            return Err(api_error(
                ErrorCode::TooLarge,
                "a view's call carries 1 MiB at most",
            ));
        }
        let open = self
            .views
            .open()
            .get(&(window.to_owned(), page.to_owned()))
            .cloned()
            .ok_or_else(|| api_error(ErrorCode::NotFound, "the view is not open"))?;
        let local = open.local.clone();
        self.activate(&local).await.map_err(|problem| {
            api_error(
                ErrorCode::Failed,
                format!("{local} could not start: {problem}"),
            )
        })?;
        let (plugin, generation) = {
            let inner = self.inner();
            let running = inner
                .loaded
                .get(&local)
                .and_then(|loaded| loaded.running.as_ref())
                .ok_or_else(|| api_error(ErrorCode::Stopped, format!("{local} stopped")))?;
            (Arc::clone(&running.plugin), running.generation)
        };
        let invocation = self.next();
        let (sender, receiver) = oneshot::channel();
        self.inner()
            .invocations
            .insert(invocation, (local.clone(), generation, sender));
        let from = serde_json::to_value(described(window, page, &open)).unwrap_or(Value::Null);
        plugin.view_call(invocation, name, args, from);
        match self.asked.within(COMMAND_WAIT, receiver).await {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(api_error(
                ErrorCode::Stopped,
                format!("{local} stopped while it answered {name}"),
            )),
            None => {
                self.inner().invocations.remove(&invocation);
                Err(api_error(
                    ErrorCode::TimedOut,
                    format!(
                        "{local} did not answer {name} in {} seconds",
                        COMMAND_WAIT.as_secs()
                    ),
                ))
            }
        }
    }

    /// `views.post`: sends an event to the plugin's own open views, by instance or by
    /// the view's name. Pages are told at once, in the order the code posted.
    pub(super) fn view_post(&self, local: &str, post: ViewsPost) -> Result<Value, ApiError> {
        if post.event.is_empty() || post.event.len() > NAME_MAX {
            return Err(api_error(
                ErrorCode::Invalid,
                "an event's name is 1 to 128 characters",
            ));
        }
        let payload = post.payload.unwrap_or(Value::Null);
        if !fits(&payload, PAYLOAD_MAX) {
            return Err(api_error(
                ErrorCode::TooLarge,
                "an event carries 1 MiB at most",
            ));
        }
        let targets: Vec<(String, String)> = self
            .views
            .open()
            .iter()
            .filter(|((window, page), open)| {
                open.local == local
                    && (instance_of(window, page) == post.to || open.view == post.to)
            })
            .map(|(key, _)| key.clone())
            .collect();
        if targets.is_empty() {
            return Ok(Value::Null);
        }
        let links = self.state.links();
        // One notice for every view it goes to, with only the view's name changed: what
        // it carries is not copied for each.
        let mut notice = json!({
            "type": "view_event",
            "view": "",
            "event": post.event,
            "payload": payload,
        });
        for (window, page) in targets {
            if let Some((_, link)) = links.iter().find(|(label, _)| *label == window) {
                notice["view"] = Value::String(page);
                link.notify(&notice);
            }
        }
        Ok(Value::Null)
    }

    /// `views.instances`: the plugin's open views, of one view or all.
    pub(super) fn view_instances(&self, local: &str, view: Option<&str>) -> Vec<ViewInstance> {
        let mut found: Vec<ViewInstance> = self
            .views
            .open()
            .iter()
            .filter(|(_, open)| open.local == local && view.is_none_or(|view| open.view == view))
            .map(|((window, page), open)| described(window, page, open))
            .collect();
        found.sort_by(|a, b| a.instance.cmp(&b.instance));
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_measured_as_the_json_it_would_be() {
        let value = json!({ "text": "a\"b", "list": [1, 2, 3] });
        let length = serde_json::to_string(&value).unwrap().len();
        assert!(fits(&value, length));
        assert!(!fits(&value, length - 1));
        assert!(fits(&Value::Null, 4));
        assert!(!fits(&Value::String("x".repeat(100)), 50));
    }

    #[test]
    fn a_page_names_its_views_plainly() {
        assert!(page_name_ok("v12"));
        assert!(page_name_ok("pane-3_a"));
        assert!(!page_name_ok(""));
        assert!(!page_name_ok("w-1/v1"));
        assert!(!page_name_ok("a b"));
        assert!(!page_name_ok(&"x".repeat(65)));
        assert_eq!(instance_of("w-1", "v3"), "w-1/v3");
        assert_eq!(Place::parse("pane"), Some(Place::Pane));
        assert_eq!(Place::parse("window"), None);
    }
}
