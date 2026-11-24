//! Showing a session of a Rex server because the application was started for it: on the
//! command line, or by a setting.
//!
//! The application's own workspace is on show first and stays so if the server cannot be
//! shown. What it takes to sign in is asked in the window, as it is when the user
//! connects from there.

use std::sync::Arc;
use std::time::Duration;

use lume_layout::WindowId;
use lume_rex::Endpoint;
use serde_json::{Value, json};

use super::hosts::GIVEN_UP;
use crate::error::CommandError;
use crate::services::{Asker, Services};
use crate::window::label_of;

/// Who says which server is to be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Said {
    /// Whoever started the application.
    Here,
    /// The settings.
    Settings,
}

/// The server a start is for: the one named on the command line, nothing being the one of
/// this machine, or the one the settings name if the start is for nothing in particular.
pub fn wanted(asked: Option<&str>, for_nothing: bool, setting: &str) -> Option<(String, Said)> {
    let setting = setting.trim();
    match asked.map(str::trim) {
        Some("") => Some((Endpoint::Local.to_string(), Said::Here)),
        Some(host) => Some((host.to_owned(), Said::Here)),
        None if for_nothing && !setting.is_empty() => Some((setting.to_owned(), Said::Settings)),
        None => None,
    }
}

/// Whether text is written as an address is, and not as what a server is called.
fn is_address(text: &str) -> bool {
    let written = text.contains("://") || text.contains(['.', ':', '@', '/']);
    written && Endpoint::parse(text).is_ok()
}

/// Shows a session of a server, beside whatever else goes on.
pub fn begin(services: &Arc<Services>, host: String, said: Said, page_wait: Duration) {
    let opening = Arc::clone(services);
    services.state.runtime().spawn(async move {
        open(&opening, &host, said, page_wait).await;
    });
}

/// Runs something in the window's page. That it could not be is not worth a word: the
/// user closed the window.
async fn in_page(services: &Services, window: WindowId, id: &str, args: Value) {
    let ran = services
        .run_command(window, id, &args, Asker::Outside)
        .await;
    if let Err(error) = ran {
        tracing::debug!(%error, id, "the window did not show what a server needs");
    }
}

async fn open(services: &Arc<Services>, host: &str, said: Said, page_wait: Duration) {
    let Some(window) = services.panes.focused_window() else {
        return;
    };
    // Whatever is asked of the user is asked in its page.
    let label = label_of(window);
    services
        .state
        .soon(page_wait, |state| state.page_of(&label))
        .await;
    let args = json!({ "host": host });
    let opened = services
        .run_command(window, "lume.rex.open", &args, Asker::Page)
        .await;
    let Err(error) = opened else { return };
    tracing::info!(%error, host, "the server the application was started for is not on show");
    match &*error.code {
        // The user did that themselves.
        GIVEN_UP | "disconnected" => {}
        "no_session" => in_page(services, window, "lume.connect.open", args).await,
        "not_found" if said == Said::Here && is_address(host) => {
            let args = json!({ "kind": "rex", "address": host });
            in_page(services, window, "lume.connection.add", args).await;
        }
        code => tell(services, host, said, code, &error),
    }
}

/// Says why a server is not on show, with a way to try again where that may help.
fn tell(services: &Services, host: &str, said: Said, code: &str, error: &CommandError) {
    let name = services
        .rex
        .label_of(host)
        .unwrap_or_else(|| host.to_owned());
    let message = match (code, said) {
        ("not_found", Said::Settings) => format!(
            "The settings name a Rex server to start on, {name}, which is not listed any more."
        ),
        ("not_found", Said::Here) => format!("There is no Rex server called {name}."),
        _ => format!("{name} could not be shown: {}.", error.message),
    };
    let again = (code != "not_found").then(|| {
        json!({
            "label": "Try again",
            "command": "lume.rex.open",
            "args": { "host": host },
        })
    });
    services.panes.notice_with("warning", message, again);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_is_for_the_server_that_is_named_or_the_settings_name() {
        let here = |host: &str| Some((host.to_owned(), Said::Here));
        assert_eq!(wanted(None, true, ""), None);
        assert_eq!(wanted(None, true, "  "), None);
        assert_eq!(wanted(Some(""), true, "ssh://build"), here("local"));
        assert_eq!(wanted(Some("build"), false, "ssh://nas"), here("build"));
        assert_eq!(
            wanted(None, true, "ssh://build"),
            Some(("ssh://build".to_owned(), Said::Settings))
        );
        // A workspace or a program was named: the start is for that.
        assert_eq!(wanted(None, false, "ssh://build"), None);
    }

    #[test]
    fn an_address_is_told_from_what_a_server_is_called() {
        for address in [
            "ssh://me@build",
            "https://build",
            "build.example",
            "build:7297",
        ] {
            assert!(is_address(address), "{address}");
        }
        for label in ["build", "Build box", "local", "ftp://x", "a.b c", ""] {
            assert!(!is_address(label), "{label}");
        }
    }
}
