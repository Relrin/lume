//! The token that proves a command comes from the application's own page.
//!
//! Plugins' panels and pane views are pages inside sandboxed frames of the same webview.
//! Whatever a frame could do, it must never be able to call the application's commands
//! as the page does. On top of the sandbox and each frame's content security policy, the
//! application gives each page it loads a secret when the page asks for it — once: the
//! first request for it after the page starts loading gets it, and every later request
//! is refused. The page keeps the secret where no frame can read it and sends it with
//! every command; a command without it is refused before it runs, and logged.
//!
//! The application's page asks first: it does so before it makes any frame.

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::Runtime;
use tauri::ipc::Invoke;

/// The header a command's token travels in.
pub const HEADER: &str = "lume-frame";

/// The token of each webview's page, by the webview's label.
#[derive(Default)]
pub struct FrameTokens {
    pages: Mutex<HashMap<String, Slot>>,
}

struct Slot {
    token: String,
    /// Whether the page has been given it; it is given once.
    given: bool,
}

/// Why the token was not handed out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The page was given it already.
    Given,
    /// The webview has no page loading.
    NoPage,
}

impl FrameTokens {
    /// A page started loading in `label`: it gets a new token, and the old page's is
    /// no longer accepted.
    pub fn page_started(&self, label: &str) {
        let slot = Slot {
            token: fresh(),
            given: false,
        };
        lock(&self.pages).insert(label.to_owned(), slot);
    }

    /// The webview is gone.
    pub fn forget(&self, label: &str) {
        lock(&self.pages).remove(label);
    }

    /// Hands the page in `label` its token, the first time it asks.
    pub fn take(&self, label: &str) -> Result<String, Refusal> {
        let mut pages = lock(&self.pages);
        let slot = pages.get_mut(label).ok_or(Refusal::NoPage)?;
        if slot.given {
            return Err(Refusal::Given);
        }
        slot.given = true;
        Ok(slot.token.clone())
    }

    /// Whether `token` is the page in `label`'s.
    pub fn accepts(&self, label: &str, token: Option<&str>) -> bool {
        let pages = lock(&self.pages);
        match (pages.get(label), token) {
            (Some(slot), Some(token)) => {
                slot.given && same(slot.token.as_bytes(), token.as_bytes())
            }
            _ => false,
        }
    }

    /// Lets a command run only when it carries its page's token, and answers it with a
    /// refusal otherwise. Gives whether the command was dealt with here (refused).
    pub fn refuses<R: Runtime>(&self, invoke: &Invoke<R>) -> bool {
        let command = invoke.message.command();
        if crate::command_registry::is_open(command) {
            return false;
        }
        let label = invoke.message.webview().label().to_owned();
        let token = invoke
            .message
            .headers()
            .get(HEADER)
            .and_then(|value| value.to_str().ok());
        if self.accepts(&label, token) {
            return false;
        }
        tracing::warn!(
            webview = %label,
            command,
            carried = token.is_some(),
            "a command without its page's token was refused"
        );
        true
    }
}

/// Compares in time that does not depend on where the two differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// 32 random bytes, as hex.
fn fresh() -> String {
    let mut bytes = [0u8; 32];
    // The system's random source does not fail on any system Lume runs on; were it to,
    // a token of zeroes would still be given only once and checked like any other.
    if let Err(error) = getrandom::fill(&mut bytes) {
        tracing::error!(%error, "no random bytes for a page's token");
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Hands the asking page its token, once per page load.
#[tauri::command]
pub fn frame_token(
    webview: tauri::Webview,
    tokens: tauri::State<'_, std::sync::Arc<FrameTokens>>,
) -> Result<String, crate::error::CommandError> {
    tokens.take(webview.label()).map_err(|refusal| {
        tracing::warn!(webview = %webview.label(), ?refusal, "a second request for a page's token was refused");
        crate::error::CommandError::forbidden("the page has its token already")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_given_its_token_once() {
        let tokens = FrameTokens::default();
        assert_eq!(tokens.take("w-1"), Err(Refusal::NoPage));
        tokens.page_started("w-1");
        let token = tokens.take("w-1").unwrap();
        assert_eq!(token.len(), 64);
        assert_eq!(tokens.take("w-1"), Err(Refusal::Given));
        assert!(tokens.accepts("w-1", Some(&token)));
        assert!(!tokens.accepts("w-1", Some("nope")));
        assert!(!tokens.accepts("w-1", None));
        assert!(!tokens.accepts("w-2", Some(&token)));
    }

    #[test]
    fn a_new_page_gets_a_new_token_and_the_old_one_stops_working() {
        let tokens = FrameTokens::default();
        tokens.page_started("w-1");
        let old = tokens.take("w-1").unwrap();
        tokens.page_started("w-1");
        assert!(!tokens.accepts("w-1", Some(&old)), "not given yet");
        let new = tokens.take("w-1").unwrap();
        assert_ne!(old, new);
        assert!(!tokens.accepts("w-1", Some(&old)));
        assert!(tokens.accepts("w-1", Some(&new)));
        tokens.forget("w-1");
        assert!(!tokens.accepts("w-1", Some(&new)));
    }

    #[test]
    fn a_token_not_yet_given_is_not_accepted() {
        let tokens = FrameTokens::default();
        tokens.page_started("w-1");
        // Someone who guessed it before the page asked gains nothing either.
        let guessed = lock(&tokens.pages).get("w-1").unwrap().token.clone();
        assert!(!tokens.accepts("w-1", Some(&guessed)));
    }
}
