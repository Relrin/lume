//! Serves the files of plugins' panels and pane views to the frames that show them.
//!
//! Requests come to `lume-plugin:` addresses (on Windows, `http://lume-plugin.localhost/`).
//! The first part of the path names the install; the rest is a path in its package. Only
//! plugins that are installed, turned on, intact and that show pages of their own are
//! served, and only files inside their packages, with the policy that holds each page
//! to its own files (see `lume_plugin::serve`).

use std::sync::{Arc, Weak};

use lume_plugin::serve::{self, Refused};
use tauri::http::{Request, Response, StatusCode};
use tauri::{Runtime, UriSchemeContext, UriSchemeResponder};

use crate::services::Services;

/// The scheme plugins' pages are served on.
pub const SCHEME: &str = "lume-plugin";

/// The address the files are served from, as pages see it, without a `/` at its end.
pub fn base() -> &'static str {
    if cfg!(windows) {
        "http://lume-plugin.localhost"
    } else {
        "lume-plugin://localhost"
    }
}

/// Where a plugin's files are, for the handler: set once the plugins are known.
#[derive(Default)]
pub struct Served {
    services: std::sync::OnceLock<Weak<Services>>,
}

impl Served {
    pub fn set(&self, services: &Arc<Services>) {
        let _ = self.services.set(Arc::downgrade(services));
    }
}

/// Answers one request.
pub fn handle<R: Runtime>(
    served: Arc<Served>,
    _context: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let path = request.uri().path().to_owned();
    let method_ok = request.method() == tauri::http::Method::GET;
    let services = served.services.get().and_then(Weak::upgrade);
    tauri::async_runtime::spawn(async move {
        let response = match (method_ok, services) {
            (false, _) => plain(StatusCode::METHOD_NOT_ALLOWED, "only GET"),
            (true, None) => plain(StatusCode::SERVICE_UNAVAILABLE, "not ready"),
            (true, Some(services)) => answer(&services, &path).await,
        };
        responder.respond(response);
    });
}

async fn answer(services: &Services, path: &str) -> Response<Vec<u8>> {
    let asked = match serve::split(path) {
        Ok(asked) => asked,
        Err(refused) => return refusal(path, &refused),
    };
    // The page itself is looked at against what was installed, as code is before it
    // starts; the files it loads come after it from the same package.
    let page = asked.path.to_ascii_lowercase().ends_with(".html");
    let Some(root) = services.plugins.frame_root(&asked.install, page).await else {
        tracing::debug!(path, "a plugin page was asked for that is not served");
        return plain(StatusCode::NOT_FOUND, "not found");
    };
    let install = asked.install.clone();
    let file_path = asked.path.clone();
    let read = tokio::task::spawn_blocking(move || {
        let file = serve::locate(&root.dir, &asked)?;
        std::fs::read(file).map_err(|_| Refused::Missing)
    })
    .await;
    match read {
        Ok(Ok(body)) => {
            let mut response = Response::builder().status(StatusCode::OK);
            for (name, value) in serve::headers(base(), &install, &file_path, root.linked) {
                response = response.header(name, value);
            }
            response
                .body(body)
                .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR, "error"))
        }
        Ok(Err(refused)) => refusal(path, &refused),
        Err(_) => plain(StatusCode::INTERNAL_SERVER_ERROR, "error"),
    }
}

fn refusal(path: &str, refused: &Refused) -> Response<Vec<u8>> {
    match refused {
        Refused::Missing => plain(StatusCode::NOT_FOUND, "not found"),
        Refused::NoInstall | Refused::BadPath => {
            tracing::warn!(path, %refused, "a plugin file was asked for by a path that was refused");
            plain(StatusCode::BAD_REQUEST, "refused")
        }
    }
}

fn plain(status: StatusCode, text: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("Content-Security-Policy", "default-src 'none'")
        .header("X-Content-Type-Options", "nosniff")
        .body(text.as_bytes().to_vec())
        .unwrap_or_default()
}

/// The folder a plugin's pages are served from.
pub struct FrameRoot {
    pub dir: std::path::PathBuf,
    /// A folder an author works on, served as it is and never cached.
    pub linked: bool,
}
