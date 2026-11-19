//! The broker: the one place a plugin's call is checked against what it was granted.
//!
//! Every call a plugin's code makes that needs a permission passes here before
//! anything is done, and the outcome is written to the audit log: the plugin, the
//! permission, the scope and whether it was allowed. What the call carried is never
//! written. A call that needs no permission (the plugin's own status items, its own
//! data) does not pass here.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::permissions::{Kind, Permissions, ScopeKind};

/// How large the audit log grows before it is set aside for a new one.
pub const AUDIT_FILE_MAX: u64 = 1024 * 1024;

/// How long the same allowed call is written once only. Refusals are always written.
pub const AUDIT_REPEAT: Duration = Duration::from_secs(60);

/// A refused call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Denied {
    pub kind: Kind,
    pub scope: Option<String>,
    pub message: String,
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Whether `granted` allows `kind` over `scope`. A kind without a scope is allowed when
/// it was granted; one with a scope when that scope was granted as written. Matching a
/// scope by its pattern is the capability's own work, which says so with
/// [`Broker::record`].
pub fn allows(granted: &Permissions, kind: Kind, scope: Option<&str>) -> bool {
    granted.required.iter().any(|request| {
        request.kind == kind
            && match (kind.scope(), scope) {
                (ScopeKind::None, _) => true,
                (_, None) => false,
                (_, Some(scope)) => request.scope.iter().any(|granted| granted == scope),
            }
    })
}

/// Whether `granted` holds `kind` at all, over any scope. A call of a kind with a scope
/// is first checked for this, and its scope is matched by the capability itself.
pub fn holds(granted: &Permissions, kind: Kind) -> bool {
    granted.required.iter().any(|request| request.kind == kind)
}

/// The grant of `kind` in `granted`, if there is one.
pub fn grant_of(granted: &Permissions, kind: Kind) -> Option<&crate::permissions::Request> {
    granted.required.iter().find(|request| request.kind == kind)
}

/// When each allowed call was last written: by plugin, permission and scope.
type Written = HashMap<(String, Kind, Option<String>), Instant>;

/// The broker, with the audit log it writes.
pub struct Broker {
    file: Option<PathBuf>,
    written: Mutex<Written>,
}

impl Broker {
    /// A broker that writes its audit log to `file`; none writes nothing.
    pub fn new(file: Option<PathBuf>) -> Self {
        Self {
            file,
            written: Mutex::new(HashMap::new()),
        }
    }

    /// Checks a call of `plugin`, with what it was granted, and records the outcome.
    pub fn check(
        &self,
        plugin: &str,
        granted: &Permissions,
        kind: Kind,
        scope: Option<&str>,
    ) -> Result<(), Denied> {
        let allowed = allows(granted, kind, scope);
        self.record(plugin, kind, scope, allowed);
        if allowed {
            Ok(())
        } else {
            let what = match scope {
                Some(scope) => format!("{kind} over {scope}"),
                None => kind.to_string(),
            };
            Err(Denied {
                kind,
                scope: scope.map(str::to_owned),
                message: format!("{plugin} has no permission for {what}"),
            })
        }
    }

    /// Writes an outcome to the audit log.
    pub fn record(&self, plugin: &str, kind: Kind, scope: Option<&str>, allowed: bool) {
        let Some(file) = &self.file else {
            return;
        };
        if allowed {
            let key = (plugin.to_owned(), kind, scope.map(str::to_owned));
            let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
            let recent = written
                .get(&key)
                .is_some_and(|at| at.elapsed() < AUDIT_REPEAT);
            if recent {
                return;
            }
            written.insert(key, Instant::now());
        }
        let line = serde_json::json!({
            "time": utc_now(),
            "plugin": plugin,
            "capability": kind.as_str(),
            "scope": scope.unwrap_or(""),
            "outcome": if allowed { "allowed" } else { "denied" },
        });
        append(file, &line.to_string(), AUDIT_FILE_MAX);
    }
}

/// Appends a line to a file, setting the file aside as `<name>.1` first when it has
/// grown past `max`.
pub fn append(file: &std::path::Path, line: &str, max: u64) {
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(file).is_ok_and(|metadata| metadata.len() > max) {
        let mut aside = file.as_os_str().to_owned();
        aside.push(".1");
        let _ = std::fs::rename(file, aside);
    }
    if let Ok(mut out) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
    {
        let _ = writeln!(out, "{line}");
    }
}

/// The time now, as `2026-11-19T08:30:05Z`.
pub fn utc_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    utc(seconds)
}

/// A time in seconds since 1970 as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn utc(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::parse_table;
    use crate::store::testing::TempDir;

    #[test]
    fn a_time_is_written_in_utc() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc(1_795_000_000), "2026-11-18T11:06:40Z");
    }

    #[test]
    fn a_call_is_allowed_only_with_its_grant_and_every_refusal_is_written() {
        let dir = TempDir::new("broker");
        let file = dir.path().join("audit.log");
        let broker = Broker::new(Some(file.clone()));
        let granted = parse_table(
            &"\"workspace.read\" = true\n\"process.run\" = [\"git\"]\n"
                .parse()
                .unwrap(),
        )
        .unwrap();
        assert!(
            broker
                .check("a.x", &granted, Kind::WorkspaceRead, None)
                .is_ok()
        );
        assert!(
            broker
                .check("a.x", &granted, Kind::WorkspaceRead, None)
                .is_ok()
        );
        assert!(
            broker
                .check("a.x", &granted, Kind::ProcessRun, Some("git"))
                .is_ok()
        );
        let denied = broker
            .check("a.x", &granted, Kind::ProcessRun, Some("sh"))
            .unwrap_err();
        assert_eq!(
            denied.message,
            "a.x has no permission for process.run over sh"
        );
        broker
            .check("a.x", &granted, Kind::PanesRead, None)
            .unwrap_err();
        broker
            .check("a.x", &granted, Kind::PanesRead, None)
            .unwrap_err();

        let lines: Vec<serde_json::Value> = std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        // The repeated allowed call once, every refusal each time.
        let outcomes: Vec<(&str, &str)> = lines
            .iter()
            .map(|line| {
                (
                    line["capability"].as_str().unwrap(),
                    line["outcome"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            outcomes,
            [
                ("workspace.read", "allowed"),
                ("process.run", "allowed"),
                ("process.run", "denied"),
                ("panes.read", "denied"),
                ("panes.read", "denied"),
            ]
        );
        assert!(
            lines
                .iter()
                .all(|line| line.as_object().unwrap().len() == 5)
        );
    }

    #[test]
    fn a_file_that_grows_too_large_is_set_aside() {
        let dir = TempDir::new("append");
        let file = dir.path().join("log");
        append(&file, &"x".repeat(100), 50);
        append(&file, "second", 50);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "second\n");
        assert!(dir.path().join("log.1").is_file());
    }
}
