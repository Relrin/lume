//! What a plugin puts to the user: a permission it asks for while it runs, a use of one
//! that the user approves each time, a question, a choice.
//!
//! Each is drawn by the page of the window in front, under the plugin's name, from
//! plain data: the plugin cannot draw, fill in or dismiss it, and what it wrote is shown
//! as text, cut to a length. A question nobody can be shown (there is no window) or
//! that goes unanswered is taken as a no.

use std::time::Duration;

use lume_plugin::broker::{allows, grant_of};
use lume_plugin::permissions::{self, Kind, Prompt, Request};
use lume_plugin_api::types::{PermissionsRequest, PickItem, UiConfirm, UiPick};
use lume_plugin_api::{ApiError, ErrorCode};
use lume_proto::app::{AppReplyBody, PageRequestBody};
use serde_json::{Value, json};

use super::{Plugins, api_error};
use crate::window::label_of;

/// How long a question put to the user waits for an answer.
const ASK_WAIT: Duration = Duration::from_secs(10 * 60);

/// The longest command or text a user is asked to approve. What is approved is shown
/// whole, so what would not fit is refused rather than shown in part.
pub(super) const APPROVE_TEXT_MAX: usize = 4000;

/// How long the words a plugin puts to the user may be, in characters.
const TITLE_MAX: usize = 80;
const MESSAGE_MAX: usize = 1000;
const REASON_MAX: usize = 300;
const LABEL_MAX: usize = 120;
const DETAIL_MAX: usize = 200;
const PICK_ITEMS_MAX: usize = 500;

/// Plain text of at most `max` characters, as a plugin's words are shown: control
/// characters other than line breaks become spaces.
pub(super) fn plain(text: &str, max: usize) -> String {
    let mut out: String = text
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .take(max)
        .collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

fn too_long(what: &str, max: usize) -> ApiError {
    api_error(
        ErrorCode::Invalid,
        format!("{what} is longer than {max} characters"),
    )
}

impl Plugins {
    /// Puts a question to the user in the window in front, on behalf of `local`, and
    /// gives back their answer.
    async fn ask_user(&self, local: &str, mut ask: Value) -> Result<Value, ApiError> {
        let services = self
            .services()
            .ok_or_else(|| api_error(ErrorCode::Failed, "there is no window to ask in"))?;
        let window = services
            .panes
            .focused_window()
            .ok_or_else(|| api_error(ErrorCode::Failed, "there is no window to ask in"))?;
        let page = self
            .state
            .page_of(&label_of(window))
            .ok_or_else(|| api_error(ErrorCode::Failed, "the window in front shows nothing yet"))?;
        let name = self
            .inner()
            .loaded
            .get(local)
            .map(|loaded| loaded.installed.name())
            .unwrap_or_else(|| local.to_owned());
        ask["plugin"] = json!({ "id": local, "name": name });
        match page.ask(PageRequestBody::PluginAsk { ask }, ASK_WAIT).await {
            Ok(AppReplyBody::Answered { answer }) => Ok(answer),
            Ok(_) => Err(api_error(
                ErrorCode::Failed,
                "the window gave an answer that does not fit the question",
            )),
            Err(error) => Err(api_error(
                ErrorCode::Failed,
                format!("the user could not be asked: {}", error.message),
            )),
        }
    }

    /// Has the user approve one use of a permission held with `prompt = "each"`,
    /// showing them `text` whole: the command that would run, or the text that would be
    /// typed, with `place` saying where. Nothing is asked for a permission held
    /// otherwise.
    pub(super) async fn approve_use(
        &self,
        local: &str,
        kind: Kind,
        text: &str,
        place: Option<String>,
    ) -> Result<(), ApiError> {
        let each = {
            let inner = self.inner();
            inner
                .loaded
                .get(local)
                .and_then(|loaded| grant_of(&loaded.grants, kind))
                .is_some_and(|grant| grant.prompt == Prompt::Each)
        };
        if !each {
            return Ok(());
        }
        if text.chars().count() > APPROVE_TEXT_MAX {
            return Err(api_error(
                ErrorCode::TooLarge,
                format!(
                    "what needs approving is longer than {APPROVE_TEXT_MAX} characters, too long \
                     to show"
                ),
            ));
        }
        let answer = self
            .ask_user(
                local,
                json!({
                    "kind": "each",
                    "permission": kind.as_str(),
                    "text": text,
                    "place": place,
                }),
            )
            .await?;
        if answer["allow"] == true {
            Ok(())
        } else {
            Err(api_error(
                ErrorCode::Denied,
                format!("the user did not allow this use of {kind}"),
            ))
        }
    }

    /// Asks the user for a permission the plugin's manifest lists as one it may ask for
    /// later, and grants it if they agree. A permission already held needs no asking.
    pub(super) async fn request_permission(
        &self,
        local: &str,
        asked: PermissionsRequest,
    ) -> Result<bool, ApiError> {
        let kind = Kind::parse(&asked.permission).ok_or_else(|| {
            api_error(
                ErrorCode::Invalid,
                format!("`{}` is not a permission", asked.permission),
            )
        })?;
        let (optional, granted) = {
            let inner = self.inner();
            let loaded = inner
                .loaded
                .get(local)
                .ok_or_else(|| api_error(ErrorCode::Failed, format!("{local} is not loaded")))?;
            let optional = loaded.manifest().and_then(|manifest| {
                manifest
                    .requested
                    .optional
                    .iter()
                    .find(|optional| optional.kind == kind)
                    .cloned()
            });
            (optional, loaded.grants.clone())
        };
        let optional = optional.ok_or_else(|| {
            api_error(
                ErrorCode::Denied,
                format!(
                    "{kind} is not among the permissions the manifest may ask for later \
                     ([permissions.optional])"
                ),
            )
        })?;
        // The scope, checked as a manifest's would be.
        let request: Request = permissions::request(kind, asked.scope.unwrap_or_default())
            .map_err(|problem| api_error(ErrorCode::Invalid, problem))?;
        if let Some(within) = &optional.within
            && let Some(outside) = request.scope.iter().find(|item| !within.contains(item))
        {
            return Err(api_error(
                ErrorCode::Denied,
                format!(
                    "`{outside}` is not within what the manifest may ask for: {}",
                    within.join(", ")
                ),
            ));
        }
        let held = if request.scope.is_empty() {
            allows(&granted, kind, None)
        } else {
            request
                .scope
                .iter()
                .all(|item| allows(&granted, kind, Some(item)))
        };
        if held {
            return Ok(true);
        }
        let reason = asked.reason.unwrap_or_default();
        if reason.chars().count() > REASON_MAX {
            return Err(too_long("the reason", REASON_MAX));
        }
        let described = permissions::describe(&request);
        let answer = self
            .ask_user(
                local,
                json!({
                    "kind": "permission",
                    "permission": kind.as_str(),
                    "what": described.what,
                    "warning": described.warning,
                    "reason": plain(&reason, REASON_MAX),
                }),
            )
            .await?;
        let allowed = answer["allow"] == true;
        let scope = request.scope.join(", ");
        self.broker.record(
            local,
            kind,
            (!scope.is_empty()).then_some(scope.as_str()),
            allowed,
        );
        if !allowed {
            return Ok(false);
        }
        let store = self.store.clone();
        let owned = local.to_owned();
        let kept = request.clone();
        tokio::task::spawn_blocking(move || store.grant(&owned, &kept))
            .await
            .map_err(|error| api_error(ErrorCode::Failed, error.to_string()))?
            .map_err(|error| api_error(ErrorCode::Failed, error.to_string()))?;
        // Held at once, before the lock file is read again.
        if let Some(loaded) = self.inner().loaded.get_mut(local) {
            match loaded
                .grants
                .required
                .iter_mut()
                .find(|held| held.kind == kind)
            {
                Some(held) => {
                    for item in &request.scope {
                        if !held.scope.contains(item) {
                            held.scope.push(item.clone());
                        }
                    }
                    held.scope.sort();
                }
                None => loaded.grants.required.push(request),
            }
        }
        Ok(true)
    }

    /// Asks the user to agree that `local` uses the services of the install `bound`
    /// for the plugin its manifest calls `wanted`: when its grant was given for another
    /// install, or for none.
    pub(super) async fn approve_binding(
        &self,
        local: &str,
        wanted: &str,
        bound: &str,
    ) -> Result<(), ApiError> {
        let bound_name = self
            .inner()
            .loaded
            .get(bound)
            .map(|loaded| loaded.installed.name())
            .unwrap_or_else(|| bound.to_owned());
        let answer = self
            .ask_user(
                local,
                json!({
                    "kind": "permission",
                    "permission": Kind::ServicesUse.as_str(),
                    "what": format!(
                        "Use what {bound_name} ({bound}) offers, in place of {wanted}"
                    ),
                    "warning": null,
                    "reason": "",
                }),
            )
            .await?;
        let allowed = answer["allow"] == true;
        self.broker
            .record(local, Kind::ServicesUse, Some(wanted), allowed);
        if !allowed {
            return Err(api_error(
                ErrorCode::Denied,
                format!("the user did not allow {local} to use {bound}"),
            ));
        }
        let store = self.store.clone();
        let (owned, wanted, bound) = (local.to_owned(), wanted.to_owned(), bound.to_owned());
        tokio::task::spawn_blocking(move || store.pin_service(&owned, &wanted, &bound))
            .await
            .map_err(|error| api_error(ErrorCode::Failed, error.to_string()))?
            .map_err(|error| api_error(ErrorCode::Failed, error.to_string()))
    }

    /// Asks the user a plugin's question, and says whether they went ahead.
    pub(super) async fn confirm(&self, local: &str, asked: UiConfirm) -> Result<bool, ApiError> {
        if asked.title.chars().count() > TITLE_MAX {
            return Err(too_long("the title", TITLE_MAX));
        }
        if asked.message.chars().count() > MESSAGE_MAX {
            return Err(too_long("the message", MESSAGE_MAX));
        }
        let confirm = asked.options.confirm.unwrap_or_else(|| "OK".to_owned());
        if confirm.chars().count() > TITLE_MAX {
            return Err(too_long("the button's label", TITLE_MAX));
        }
        let answer = self
            .ask_user(
                local,
                json!({
                    "kind": "confirm",
                    "title": plain(&asked.title, TITLE_MAX),
                    "message": plain(&asked.message, MESSAGE_MAX),
                    "confirm": plain(&confirm, TITLE_MAX),
                    "danger": asked.options.danger.unwrap_or(false),
                }),
            )
            .await?;
        Ok(answer["ok"] == true)
    }

    /// Has the user choose one of a plugin's items: its id, or none.
    pub(super) async fn pick(
        &self,
        local: &str,
        asked: UiPick,
    ) -> Result<Option<String>, ApiError> {
        if asked.items.is_empty() {
            return Ok(None);
        }
        if asked.items.len() > PICK_ITEMS_MAX {
            return Err(api_error(
                ErrorCode::TooLarge,
                format!("a pick offers {PICK_ITEMS_MAX} items at most"),
            ));
        }
        let items: Vec<PickItem> = asked
            .items
            .iter()
            .map(|item| PickItem {
                id: item.id.clone(),
                label: plain(&item.label, LABEL_MAX),
                detail: item
                    .detail
                    .as_deref()
                    .map(|detail| plain(detail, DETAIL_MAX)),
            })
            .collect();
        let answer = self
            .ask_user(
                local,
                json!({
                    "kind": "pick",
                    "title": asked.options.title.as_deref().map(|title| plain(title, TITLE_MAX)),
                    "placeholder": asked
                        .options
                        .placeholder
                        .as_deref()
                        .map(|text| plain(text, TITLE_MAX)),
                    "items": items,
                }),
            )
            .await?;
        Ok(answer["chosen"]
            .as_str()
            .filter(|chosen| asked.items.iter().any(|item| item.id == *chosen))
            .map(str::to_owned))
    }
}
