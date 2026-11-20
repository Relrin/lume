//! Opening a file or an address with a plugin's view.
//!
//! A plugin's manifest may say that one of its pane views opens files whose names match
//! a pattern, or addresses of a scheme. Whatever is to be opened is matched against
//! those here, from its text alone: a page that asks is not trusted to have matched it.
//! One view that matches, where nothing else would open the thing, opens it. Where
//! there is a choice (several views, or a view and what the system would do with it),
//! the user makes it, in a list the window draws.
//!
//! A view is told what to open in the state it starts with: `{ "open": { "path": … } }`
//! for a file, `{ "open": { "uri": … } }` for an address. Being told a path gives a
//! plugin no leave to read it: that is still its permissions' to say.

use std::path::PathBuf;
use std::sync::Arc;

use lume_layout::WindowId;
use lume_plugin::contributions::OpenerDef;
use lume_plugin_api::types::OpenViewOptions;
use lume_proto::app::{AppReplyBody, PageRequestBody};
use serde_json::{Value, json};
use url::Url;

use super::Plugins;
use crate::error::CommandError;
use crate::opener;
use crate::tuning::LINK_MAX_BYTES;
use crate::window::label_of;

/// The choice that leaves the thing to the system.
const NORMALLY: &str = "lume.normally";

/// How long the user has to choose.
const CHOOSE_WAIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// What is to be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    /// A file on this machine, by its whole path.
    File(PathBuf),
    /// An address that is not a file's.
    Address(Url),
}

impl Asked {
    /// Reads what a command was given: a `path`, or a `uri`.
    pub fn read(args: &Value) -> Result<Self, CommandError> {
        let text = |key: &str| args.get(key).and_then(Value::as_str);
        match (text("path"), text("uri")) {
            (Some(path), None) => {
                let path = PathBuf::from(path);
                if path.as_os_str().len() > LINK_MAX_BYTES || !path.is_absolute() {
                    return Err(CommandError::bad_request(
                        "a file is opened by its whole path",
                    ));
                }
                Ok(Self::File(path))
            }
            (None, Some(uri)) => {
                if uri.len() > LINK_MAX_BYTES {
                    return Err(CommandError::bad_request("the address is too long"));
                }
                let url = Url::parse(uri).map_err(|error| {
                    CommandError::bad_request(format!("not an address: {error}"))
                })?;
                if url.scheme() != "file" {
                    return Ok(Self::Address(url));
                }
                // A file on another machine is not looked at, as a link to one is not.
                if !matches!(url.host_str(), None | Some("" | "localhost")) {
                    return Err(CommandError::new(
                        "unsupported",
                        "files on other machines are not opened",
                    ));
                }
                let mut local = url;
                let _ = local.set_host(None);
                local.to_file_path().map(Self::File).map_err(|()| {
                    CommandError::bad_request("the address is not a path on this machine")
                })
            }
            _ => Err(CommandError::bad_request(
                "what to open is said as a `path` or as a `uri`, one of them",
            )),
        }
    }

    /// What a view is told to open.
    fn state(&self) -> Value {
        match self {
            Self::File(path) => json!({ "open": { "path": path.to_string_lossy() } }),
            Self::Address(url) => json!({ "open": { "uri": url.as_str() } }),
        }
    }

    /// The address the system would be given.
    fn link(&self) -> Option<String> {
        match self {
            Self::File(path) => Url::from_file_path(path).ok().map(String::from),
            Self::Address(url) => Some(url.as_str().to_owned()),
        }
    }

    /// What it is called where the user is asked about it.
    fn shown(&self) -> String {
        match self {
            Self::File(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
            Self::Address(url) => url.as_str().to_owned(),
        }
    }
}

/// Whether a file's name fits a pattern: the pattern's letters as they are, whatever
/// their case, with `*` for any run of characters.
pub fn fits(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    // The place in each, and where to go back to after the last `*`.
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if p < pattern.len() && pattern[p] == name[n] {
            p += 1;
            n += 1;
        } else if let Some((at, taken)) = star {
            p = at + 1;
            n = taken + 1;
            star = Some((at, taken + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// The views that open `asked`, in the order the plugins give them.
pub fn openers_for<'a>(openers: &'a [OpenerDef], asked: &Asked) -> Vec<&'a OpenerDef> {
    openers
        .iter()
        .filter(|opener| match asked {
            Asked::File(path) => path
                .file_name()
                .map(|name| name.to_string_lossy())
                .is_some_and(|name| opener.patterns.iter().any(|pattern| fits(pattern, &name))),
            Asked::Address(url) => opener
                .schemes
                .iter()
                .any(|scheme| scheme.eq_ignore_ascii_case(url.scheme())),
        })
        .collect()
}

fn id_of(opener: &OpenerDef) -> String {
    format!("{}.{}", opener.plugin, opener.view)
}

/// Hands a file or an address to the system, away from the async threads.
async fn open_normally(link: String) -> Result<(), CommandError> {
    tokio::task::spawn_blocking(move || opener::open(&link))
        .await
        .map_err(|error| CommandError::internal(error.to_string()))?
}

impl Plugins {
    /// Opens a file or an address in `window`: with the view a plugin offers for it,
    /// with what the system does with it, or with whichever of those the user chooses
    /// when there is more than one. `args` names it as a `path` or a `uri`, and may
    /// name the view to use as `view` (`<plugin>.<view>`).
    pub async fn open_target(
        self: &Arc<Self>,
        window: WindowId,
        args: &Value,
    ) -> Result<(), CommandError> {
        let asked = Asked::read(args)?;
        let offered: Vec<OpenerDef> = {
            let inner = self.inner();
            openers_for(&inner.shown.contributions.openers, &asked)
                .into_iter()
                .cloned()
                .collect()
        };
        // What the system would do with it, where it would do anything.
        let normal = asked.link().filter(|link| opener::resolve(link).is_ok());
        let chosen = match args.get("view").and_then(Value::as_str) {
            Some(named) => offered
                .iter()
                .find(|opener| id_of(opener) == named)
                .map(id_of)
                .ok_or_else(|| {
                    CommandError::not_found(format!("{named} does not open {}", asked.shown()))
                })?,
            None => match (offered.as_slice(), &normal) {
                ([], None) => {
                    return Err(CommandError::new(
                        "unsupported",
                        format!("nothing here opens {}", asked.shown()),
                    ));
                }
                ([], Some(_)) => NORMALLY.to_owned(),
                ([only], None) => id_of(only),
                _ => match self
                    .choose(window, &asked, &offered, normal.is_some())
                    .await?
                {
                    Some(chosen) => chosen,
                    // Put away without a choice: nothing is opened.
                    None => return Ok(()),
                },
            },
        };
        if chosen == NORMALLY {
            return match normal {
                Some(link) => open_normally(link).await,
                None => Err(CommandError::new(
                    "unsupported",
                    "the system does not open it",
                )),
            };
        }
        let opener = offered
            .iter()
            .find(|opener| id_of(opener) == chosen)
            .ok_or_else(|| CommandError::internal("the choice was not one of those offered"))?;
        let title = match &asked {
            Asked::File(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Asked::Address(_) => None,
        };
        self.open_view(
            Some(window),
            &opener.plugin,
            &opener.view,
            OpenViewOptions {
                state: Some(asked.state()),
                title,
                ..OpenViewOptions::default()
            },
        )
        .await
        .map(drop)
    }

    /// Has the user choose what opens `asked`, in the page of `window`. Nothing when
    /// they chose none.
    async fn choose(
        &self,
        window: WindowId,
        asked: &Asked,
        offered: &[OpenerDef],
        normal: bool,
    ) -> Result<Option<String>, CommandError> {
        let page = self.state.page_of(&label_of(window)).ok_or_else(|| {
            CommandError::new(
                lume_proto::app::codes::NO_WINDOW,
                "the window shows nothing yet, so there is nowhere to choose",
            )
        })?;
        let mut items: Vec<Value> = {
            let inner = self.inner();
            offered
                .iter()
                .map(|opener| {
                    let plugin = inner
                        .loaded
                        .get(&opener.plugin)
                        .map(|loaded| loaded.installed.name())
                        .unwrap_or_else(|| opener.plugin.clone());
                    json!({ "id": id_of(opener), "label": opener.title, "detail": plugin })
                })
                .collect()
        };
        if normal {
            let detail = match asked {
                Asked::File(_) => "Shown in the file manager",
                Asked::Address(_) => "The system's own way",
            };
            items.push(json!({ "id": NORMALLY, "label": "Open normally", "detail": detail }));
        }
        let ask = json!({
            "kind": "pick",
            "plugin": { "id": "lume", "name": "Open with" },
            "title": super::asking::plain(&asked.shown(), 80),
            "placeholder": null,
            "items": items,
        });
        match page
            .ask(PageRequestBody::PluginAsk { ask }, CHOOSE_WAIT)
            .await?
        {
            AppReplyBody::Answered { answer } => Ok(answer["chosen"]
                .as_str()
                .filter(|chosen| {
                    (normal && *chosen == NORMALLY)
                        || offered.iter().any(|opener| id_of(opener) == *chosen)
                })
                .map(str::to_owned)),
            _ => Err(CommandError::internal(
                "the window gave an answer that does not fit a choice",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opener(view: &str, patterns: &[&str], schemes: &[&str]) -> OpenerDef {
        OpenerDef {
            plugin: "acme.review".to_owned(),
            view: view.to_owned(),
            title: view.to_owned(),
            patterns: patterns.iter().map(|&pattern| pattern.to_owned()).collect(),
            schemes: schemes.iter().map(|&scheme| scheme.to_owned()).collect(),
        }
    }

    #[test]
    fn a_name_fits_a_pattern_with_stars_whatever_its_case() {
        assert!(fits("*.diff", "change.diff"));
        assert!(fits("*.diff", "CHANGE.DIFF"));
        assert!(fits("*.d*f", "a.diff"));
        assert!(fits("makefile", "Makefile"));
        assert!(fits("*", ""));
        assert!(fits("a*b*c", "a--b--b--c"));
        assert!(!fits("*.diff", "change.diff.orig"));
        assert!(!fits("*.diff", "diff"));
        assert!(!fits("a*b", "a--c"));
        assert!(!fits("", "x"));
    }

    #[test]
    fn what_is_asked_is_a_file_or_an_address_never_both() {
        let root = if cfg!(windows) { "C:\\work" } else { "/work" };
        let file = format!("{root}{}a.diff", std::path::MAIN_SEPARATOR);
        assert_eq!(
            Asked::read(&json!({ "path": file })).unwrap(),
            Asked::File(PathBuf::from(&file))
        );
        let link = Url::from_file_path(&file).unwrap();
        assert_eq!(
            Asked::read(&json!({ "uri": link.as_str() })).unwrap(),
            Asked::File(PathBuf::from(&file))
        );
        assert!(matches!(
            Asked::read(&json!({ "uri": "acme-review://change/12" })).unwrap(),
            Asked::Address(_)
        ));
        for refused in [
            json!({}),
            json!({ "path": "a.diff" }),
            json!({ "path": file, "uri": "https://example.com" }),
            json!({ "uri": "not an address" }),
            json!({ "uri": "file://elsewhere/share/a.diff" }),
        ] {
            assert!(Asked::read(&refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn views_are_offered_by_the_files_name_or_the_addresss_scheme() {
        let openers = [
            opener("review", &["*.diff", "*.patch"], &["acme-review"]),
            opener("notes", &["*.md"], &[]),
        ];
        let root = if cfg!(windows) {
            "C:\\work\\"
        } else {
            "/work/"
        };
        let file = |name: &str| Asked::File(PathBuf::from(format!("{root}{name}")));
        let views = |asked: &Asked| -> Vec<String> {
            openers_for(&openers, asked)
                .into_iter()
                .map(|opener| opener.view.clone())
                .collect()
        };
        assert_eq!(views(&file("fix.PATCH")), ["review"]);
        assert_eq!(views(&file("readme.md")), ["notes"]);
        assert!(views(&file("main.rs")).is_empty());
        // An address is matched by its scheme alone, never by how it ends.
        let address = |text: &str| Asked::Address(Url::parse(text).unwrap());
        assert_eq!(views(&address("ACME-review://x/1")), ["review"]);
        assert!(views(&address("https://example.com/a.diff")).is_empty());
        assert_eq!(
            file("a.diff").state()["open"]["path"],
            format!("{root}a.diff")
        );
        assert_eq!(
            address("acme-review://x/1").state(),
            json!({ "open": { "uri": "acme-review://x/1" } })
        );
    }
}
