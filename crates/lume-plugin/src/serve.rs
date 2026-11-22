//! How the files of a plugin's pages are served to the frames that show them.
//!
//! A plugin's panels and pane views are pages in its package, shown in sandboxed frames.
//! Every frame of every plugin is served from one host, and the first part of a file's
//! path names the install: `<base>/<install>/<path in the package>`. Each page is told,
//! through its content security policy, that it may load scripts, styles, pictures,
//! fonts and media only from below its own install's path, and that it may connect to
//! nothing at all. The sandbox gives the page an origin of its own that matches no other
//! page, so it holds no storage and cannot reach the window it is in.
//!
//! One path belongs to no install: `<base>/lume/fonts/<file>`, where the application
//! offers the fonts it ships to every page, so a plugin's view is drawn in the window's
//! own fonts without each package carrying them. `lume` can be no plugin's id.
//!
//! Nothing here reads a file: this works out what was asked for, whether it may be
//! served, and what goes with it.

use std::path::{Component, Path, PathBuf};

use crate::id;

/// Why a request is not served.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    #[error("the address names no plugin")]
    NoInstall,
    #[error("the address is not a path in the package")]
    BadPath,
    #[error("there is no such file in the package")]
    Missing,
}

/// A request's path, cut into the install it names and the file's path in its package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asked {
    pub install: String,
    /// Relative, with `/` between folders; never empty, never `..`.
    pub path: String,
}

/// Cuts the path of a request (`/<install>/<path>`, possibly with a query or a fragment,
/// percent-encoded) into the install and the path in its package. A path that could
/// leave the package however it is read is refused: `..` and `.` parts, empty parts,
/// back slashes, drive letters and other `:`, control characters, and any of these
/// spelt with percent signs (which are decoded once; a `%` left after that is refused
/// too, so nothing is decoded twice along the way).
pub fn split(url_path: &str) -> Result<Asked, Refused> {
    let path = url_path
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_start_matches('/');
    let decoded = percent_decode(path).ok_or(Refused::BadPath)?;
    let (install, rest) = decoded.split_once('/').ok_or(Refused::BadPath)?;
    if id::check_plugin_id(install).is_err() {
        return Err(Refused::NoInstall);
    }
    let bad = rest.is_empty()
        || rest.contains(['\\', ':', '%'])
        || rest.chars().any(char::is_control)
        || rest
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || reserved(part));
    if bad {
        return Err(Refused::BadPath);
    }
    Ok(Asked {
        install: install.to_owned(),
        path: rest.to_owned(),
    })
}

/// The path, below the base, that the application's own fonts are served under.
pub const SHARED_FONTS: &str = "lume/fonts";

/// The file a request for one of the application's fonts names
/// (`/lume/fonts/<file>.woff2`), or nothing if the path is not one of those. The name is
/// letters, digits and hyphens before its ending, so it can name nothing but a file.
pub fn shared_font(url_path: &str) -> Option<&str> {
    let path = url_path
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_start_matches('/');
    let name = path.strip_prefix(SHARED_FONTS)?.strip_prefix('/')?;
    let stem = name.strip_suffix(".woff2")?;
    let plain = !stem.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    plain.then_some(name)
}

/// The headers one of the application's fonts is served with. A page's frame has an
/// origin of its own, and a font is only taken from another origin that says anyone
/// may have it; these are fonts anyone may have. They never change while the
/// application runs.
pub fn shared_font_headers() -> Vec<(&'static str, String)> {
    vec![
        ("Content-Type", "font/woff2".to_owned()),
        ("Access-Control-Allow-Origin", "*".to_owned()),
        ("Cross-Origin-Resource-Policy", "cross-origin".to_owned()),
        ("X-Content-Type-Options", "nosniff".to_owned()),
        ("Cache-Control", "max-age=86400".to_owned()),
    ]
}

/// Whether a file name is one Windows keeps for a device (`con`, `nul`, `com1`, …),
/// whatever follows a dot, or ends with a dot or a space.
fn reserved(part: &str) -> bool {
    let stem = part
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let device = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    device || part.ends_with('.') || part.ends_with(' ')
}

/// Decodes `%XX` once. Text that is not UTF-8 after decoding, or a `%` not followed by
/// two hex digits, gives nothing.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = text.get(at + 1..at + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The file a request names in a package folder, once it is certain to be a plain file
/// inside it: the folder and the file are made canonical (links followed), and the file
/// must lie under the folder.
pub fn locate(root: &Path, asked: &Asked) -> Result<PathBuf, Refused> {
    let root = std::fs::canonicalize(root).map_err(|_| Refused::Missing)?;
    let mut file = root.clone();
    for part in asked.path.split('/') {
        file.push(part);
    }
    let file = std::fs::canonicalize(&file).map_err(|_| Refused::Missing)?;
    let inside = file.starts_with(&root)
        && file
            .strip_prefix(&root)
            .is_ok_and(|rest| rest.components().all(|c| matches!(c, Component::Normal(_))));
    if !inside {
        return Err(Refused::BadPath);
    }
    if !file.is_file() {
        return Err(Refused::Missing);
    }
    Ok(file)
}

/// The type a file is served as, by its extension. Anything not listed is served as
/// bytes, which a page cannot run or show as a document.
pub fn content_type(path: &str) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// The content security policy of every file served for `install`, where `base` is the
/// address frames are served from (no `/` at its end).
///
/// Scripts, pictures, fonts and media come only from below the install's own path;
/// fonts may also be the application's own (`SHARED_FONTS`).
/// Styles may also be written in the page (many interface libraries set colours and
/// sizes that way), which is safe here because a style has no way to send anything
/// out: every other kind of request is held to the install's path or refused. Workers
/// may be made from the plugin's files or from what its code makes in memory. Nothing
/// may connect anywhere, be framed, submit a form, or change the page's base address.
pub fn content_security_policy(base: &str, install: &str) -> String {
    let own = format!("{base}/{install}/");
    [
        "default-src 'none'".to_owned(),
        format!("script-src {own}"),
        format!("style-src {own} 'unsafe-inline'"),
        format!("img-src {own} data: blob:"),
        format!("font-src {own} {base}/{SHARED_FONTS}/ data:"),
        format!("media-src {own} blob:"),
        format!("worker-src {own} blob:"),
        "connect-src 'none'".to_owned(),
        "frame-src 'none'".to_owned(),
        "child-src 'none'".to_owned(),
        "object-src 'none'".to_owned(),
        "manifest-src 'none'".to_owned(),
        "base-uri 'none'".to_owned(),
        "form-action 'none'".to_owned(),
    ]
    .join("; ")
}

/// The headers served with every file of `install`. `linked` plugins (folders an author
/// works on) are never cached, so a page that is shown again has their latest files.
pub fn headers(base: &str, install: &str, path: &str, linked: bool) -> Vec<(&'static str, String)> {
    vec![
        ("Content-Type", content_type(path).to_owned()),
        (
            "Content-Security-Policy",
            content_security_policy(base, install),
        ),
        ("X-Content-Type-Options", "nosniff".to_owned()),
        ("Cross-Origin-Resource-Policy", "same-origin".to_owned()),
        ("Referrer-Policy", "no-referrer".to_owned()),
        ("Permissions-Policy", PERMISSIONS_POLICY.to_owned()),
        (
            "Cache-Control",
            if linked { "no-store" } else { "no-cache" }.to_owned(),
        ),
    ]
}

/// What a page is never allowed to ask the system for, whatever the frame's attributes
/// say.
pub const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=(), \
    clipboard-read=(), clipboard-write=(), display-capture=(), usb=(), serial=(), hid=(), \
    bluetooth=(), payment=(), publickey-credentials-get=(), screen-wake-lock=(), \
    fullscreen=()";

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(install: &str, path: &str) -> Result<Asked, Refused> {
        Ok(Asked {
            install: install.to_owned(),
            path: path.to_owned(),
        })
    }

    #[test]
    fn a_path_is_cut_into_its_install_and_file() {
        assert_eq!(
            split("/acme.review/ui/index.html"),
            asked("acme.review", "ui/index.html")
        );
        assert_eq!(
            split("/acme.review/ui/index.html?x=1#top"),
            asked("acme.review", "ui/index.html")
        );
        assert_eq!(
            split("acme.review/a%20b.js"),
            asked("acme.review", "a b.js")
        );
    }

    #[test]
    fn a_path_that_could_leave_the_package_is_refused() {
        for path in [
            "/acme.review/../other/x.js",
            "/acme.review/ui/../../x.js",
            "/acme.review/./x.js",
            "/acme.review//x.js",
            "/acme.review/ui/",
            "/acme.review/",
            "/acme.review",
            "/acme.review/%2e%2e/x.js",
            "/acme.review/%2E%2E%2Fx.js",
            "/acme.review/ui%2f..%2f..%2fx.js",
            "/acme.review/%252e%252e/x.js",
            "/acme.review/..%5cx.js",
            "/acme.review/ui\\x.js",
            "/acme.review/C:/x.js",
            "/acme.review/c:x.js",
            "/acme.review/x%00.js",
            "/acme.review/x%0a.js",
            "/acme.review/%zz.js",
            "/acme.review/%c3%28.js",
            "/acme.review/con",
            "/acme.review/NUL.txt",
            "/acme.review/com1.js",
            "/acme.review/x.js.",
            "/acme.review/x.js%20",
        ] {
            assert_eq!(split(path), Err(Refused::BadPath), "{path}");
        }
    }

    #[test]
    fn a_path_must_name_an_install() {
        for path in [
            "/ACME/x.js",
            "/../x.js",
            "/a b/x.js",
            "/lume:x/x.js",
            "/%2e%2e/x.js",
        ] {
            assert!(split(path).is_err(), "{path}");
        }
    }

    #[test]
    fn only_plain_files_inside_the_package_are_located() {
        let dir = std::env::temp_dir().join(format!("lume-serve-{}", std::process::id()));
        let root = dir.join("package");
        std::fs::create_dir_all(root.join("ui")).unwrap();
        std::fs::write(root.join("ui").join("index.html"), "<p>hi</p>").unwrap();
        std::fs::write(dir.join("secret.txt"), "no").unwrap();
        let found = locate(&root, &asked("a", "ui/index.html").unwrap()).unwrap();
        assert!(found.ends_with(Path::new("ui").join("index.html")));
        assert_eq!(
            locate(&root, &asked("a", "ui/missing.html").unwrap()),
            Err(Refused::Missing)
        );
        assert_eq!(
            locate(&root, &asked("a", "ui").unwrap()),
            Err(Refused::Missing)
        );
        // A link out of the package is followed and found outside it. Making one may be
        // refused to this account (Windows without the right); then a junction stands in.
        let link = root.join("out");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&dir, &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&dir, &link).is_ok()
            || std::process::Command::new("cmd")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(&link)
                .arg(&dir)
                .output()
                .is_ok_and(|out| out.status.success());
        if made {
            assert_eq!(
                locate(&root, &asked("a", "out/secret.txt").unwrap()),
                Err(Refused::BadPath)
            );
        }
        let _ = std::fs::remove_dir(&link);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_policy_holds_a_page_to_its_own_files() {
        let policy = content_security_policy("http://lume-plugin.localhost", "acme.review");
        let own = "http://lume-plugin.localhost/acme.review/";
        assert!(policy.contains("default-src 'none'"));
        assert!(policy.contains(&format!("script-src {own};")));
        assert!(!policy.contains("unsafe-eval"));
        assert!(policy.contains("connect-src 'none'"));
        assert!(policy.contains("frame-src 'none'"));
        assert!(policy.contains("base-uri 'none'"));
        assert!(policy.contains("form-action 'none'"));
        // Only styles may be written in the page.
        assert_eq!(policy.matches("'unsafe-inline'").count(), 1);
        assert!(policy.contains(&format!("style-src {own} 'unsafe-inline'")));
        // Fonts are its own or the application's, and nothing else is the application's.
        assert!(policy.contains(&format!(
            "font-src {own} http://lume-plugin.localhost/lume/fonts/ data:"
        )));
        assert_eq!(policy.matches("/lume/").count(), 1);
    }

    #[test]
    fn the_applications_fonts_have_a_path_no_plugin_can_have() {
        assert_eq!(
            shared_font("/lume/fonts/InterVariable.woff2"),
            Some("InterVariable.woff2")
        );
        assert_eq!(
            shared_font("lume/fonts/JetBrainsMono-Bold.woff2?x=1"),
            Some("JetBrainsMono-Bold.woff2")
        );
        for path in [
            "/lume/fonts/",
            "/lume/fonts/x.ttf",
            "/lume/fonts/../x.woff2",
            "/lume/fonts/a/b.woff2",
            "/lume/fonts/a%2fb.woff2",
            "/lume/fonts/.woff2",
            "/lume/other/x.woff2",
            "/lume.fonts/x.woff2",
            "/acme.review/lume/fonts/x.woff2",
        ] {
            assert_eq!(shared_font(path), None, "{path}");
        }
        // The path is the application's alone: it names no install.
        assert_eq!(
            split("/lume/fonts/InterVariable.woff2"),
            Err(Refused::NoInstall)
        );
        let headers = shared_font_headers();
        assert!(headers.contains(&("Content-Type", "font/woff2".to_owned())));
        assert!(headers.contains(&("Access-Control-Allow-Origin", "*".to_owned())));
    }

    #[test]
    fn files_are_served_as_their_kind() {
        assert_eq!(content_type("ui/index.HTML"), "text/html; charset=utf-8");
        assert_eq!(content_type("a.mjs"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("a.woff2"), "font/woff2");
        assert_eq!(content_type("README"), "application/octet-stream");
        assert_eq!(content_type("x.exe"), "application/octet-stream");
        let headers = headers("lume-plugin://localhost", "a", "x.js", true);
        let get = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(get("X-Content-Type-Options"), Some("nosniff"));
        assert_eq!(get("Cache-Control"), Some("no-store"));
        assert!(
            get("Content-Security-Policy")
                .unwrap()
                .contains("lume-plugin://localhost/a/")
        );
    }
}
