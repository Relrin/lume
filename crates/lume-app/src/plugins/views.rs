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

use lume_plugin_api::types::{ViewInstance, ViewsPost};
use lume_plugin_api::{ApiError, ErrorCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::{COMMAND_WAIT, Plugins, api_error};
use crate::error::CommandError;

/// The most a view's call or an event to a view may carry, as JSON.
pub const PAYLOAD_MAX: usize = 1024 * 1024;

/// The most views a window may have open at once.
const OPEN_MAX: usize = 256;

/// The longest name a call or an event may have.
const NAME_MAX: usize = 128;

/// One open view.
#[derive(Clone, Debug)]
struct Open {
    local: String,
    view: String,
    place: Place,
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
}

impl Views {
    fn open(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), Open>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
    ) -> Result<String, CommandError> {
        if !page_name_ok(page) {
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
            self.tell_views(&open.local, "close", &described(window, page, &open));
        }
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
        if serde_json::to_string(&args).map_or(true, |text| text.len() > PAYLOAD_MAX) {
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
        match tokio::time::timeout(COMMAND_WAIT, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(api_error(
                ErrorCode::Stopped,
                format!("{local} stopped while it answered {name}"),
            )),
            Err(_) => {
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
        if serde_json::to_string(&payload).map_or(true, |text| text.len() > PAYLOAD_MAX) {
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
        for (window, page) in targets {
            if let Some((_, link)) = links.iter().find(|(label, _)| *label == window) {
                link.notify(&json!({
                    "type": "view_event",
                    "view": page,
                    "event": post.event,
                    "payload": payload,
                }));
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
