//! Which folder of a server's machine a terminal is in.
//!
//! A server says in which folder the process in a terminal's foreground is. That is the
//! system's own word, and it is what counts: what a program prints about a folder may
//! come from a machine further away, or from a file that was shown.
//!
//! One kind of shell has to be told apart. A PowerShell's process stays in the folder it
//! was started in, wherever its user goes. Of it only what it printed itself can be told,
//! and a server says which process printed a folder.
//!
//! Either is a path of the server's machine and means nothing here.

use lume_session::cwd::shown_dir;
use lume_session::program_stem;
use serde_json::Value;

use crate::api::Process;
use crate::text::plain;
use crate::tuning::{FOLDER_PRINTED_MAX_BYTES, TEXT_MAX_BYTES};

/// A process of the server's, as far as it tells where a terminal is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Program {
    pid: i64,
    name: String,
    cwd: Option<String>,
}

impl Program {
    /// The process as a server describes one.
    pub(crate) fn read(process: &Value) -> Option<Self> {
        Some(Self {
            pid: process.get("pid")?.as_i64()?,
            name: name_of(process)?,
            cwd: process
                .get("cwd")
                .and_then(Value::as_str)
                .filter(|path| path.len() <= TEXT_MAX_BYTES)
                .map(str::to_owned),
        })
    }

    /// The process as an answer gives it.
    pub(crate) fn of(process: &Process) -> Option<Self> {
        (process.name.len() <= TEXT_MAX_BYTES).then(|| Self {
            pid: process.pid,
            name: process.name.clone(),
            cwd: process
                .cwd
                .clone()
                .filter(|path| path.len() <= TEXT_MAX_BYTES),
        })
    }

    /// Whether it stays where it was started while its user goes elsewhere.
    pub(crate) fn stays(&self) -> bool {
        matches!(
            program_stem(&self.name).as_str(),
            "pwsh" | "pwsh-preview" | "powershell"
        )
    }
}

fn name_of(process: &Value) -> Option<String> {
    let name = process.get("name")?.as_str()?;
    (name.len() <= TEXT_MAX_BYTES).then(|| name.to_owned())
}

/// The folder a program printed, and which program that was by the server's word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Printed {
    text: String,
    pid: i64,
    name: String,
}

impl Printed {
    /// What was printed, as a server tells it. One whose owner is not told is nobody's.
    pub(crate) fn read(told: &Value) -> Option<Self> {
        let owner = told.get("owner")?;
        Some(Self {
            text: told
                .get("pwd")
                .and_then(Value::as_str)
                .filter(|text| text.len() <= FOLDER_PRINTED_MAX_BYTES)?
                .to_owned(),
            pid: owner.get("pid")?.as_i64()?,
            name: name_of(owner)?,
        })
    }

    fn path(&self) -> Option<String> {
        if self
            .text
            .get(..5)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
        {
            shown_dir(&self.text)
        } else {
            Some(self.text.clone())
        }
    }
}

/// Something a server told of where a terminal is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Told {
    /// The process in the foreground, where there is one.
    Front(Option<Program>),
    Printed(Option<Printed>),
}

impl Told {
    /// What an event of a terminal block tells, if it tells anything of this.
    pub(crate) fn read(name: &str, payload: &Value) -> Option<Self> {
        match name {
            "process_changed" => Some(Self::Front(
                payload.get("foreground").and_then(Program::read),
            )),
            "pwd_changed" => Some(Self::Printed(Printed::read(payload))),
            _ => None,
        }
    }
}

/// What is known of where a terminal is. Each thing is kept with the number of the last
/// event that whoever told it had sent, so that an answer that was on its way when
/// something newer was told does not take its place.
#[derive(Debug, Default)]
pub(crate) struct Whereabouts {
    front: (u64, Option<Program>),
    printed: (u64, Option<Printed>),
}

impl Whereabouts {
    pub(crate) fn note(&mut self, seq: u64, told: Told) {
        match told {
            Told::Front(front) if seq >= self.front.0 => self.front = (seq, front),
            Told::Printed(printed) if seq >= self.printed.0 => self.printed = (seq, printed),
            _ => {}
        }
    }

    /// The folder, as a path of a machine that runs Windows or does not.
    pub(crate) fn folder(&self, windows: bool) -> Option<String> {
        let front = self.front.1.as_ref()?;
        let path = if front.stays() {
            let printed = self.printed.1.as_ref()?;
            let itself = printed.pid == front.pid && printed.name == front.name;
            printed.path().filter(|_| itself)
        } else {
            front.cwd.clone()
        };
        path.filter(|path| is_folder(path, windows))
    }
}

/// Whether a path begins at a drive or the root, and is no other than it looks. A share
/// is another machine's again.
fn is_folder(path: &str, windows: bool) -> bool {
    let absolute = if windows {
        let mut chars = path.chars();
        chars
            .next()
            .is_some_and(|drive| drive.is_ascii_alphabetic())
            && chars.next() == Some(':')
            && matches!(chars.next(), Some('\\' | '/'))
    } else {
        path.starts_with('/')
    };
    absolute && path.len() <= TEXT_MAX_BYTES && plain(path) == path
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn front(name: &str, cwd: &str) -> Told {
        Told::read(
            "process_changed",
            &json!({ "foreground": { "pid": 7, "name": name, "cwd": cwd } }),
        )
        .unwrap()
    }

    fn printed(text: &str, pid: i64, name: &str) -> Told {
        let payload = json!({ "pwd": text, "owner": { "pid": pid, "name": name } });
        Told::read("pwd_changed", &payload).unwrap()
    }

    fn folder(windows: bool, told: impl IntoIterator<Item = Told>) -> Option<String> {
        let mut whereabouts = Whereabouts::default();
        for (nth, told) in (1..).zip(told) {
            whereabouts.note(nth, told);
        }
        whereabouts.folder(windows)
    }

    #[test]
    fn a_folder_is_where_the_process_in_front_is_and_not_what_was_printed() {
        let elsewhere = || printed("file://other/C:/Elsewhere", 7, "cmd.exe");
        for (windows, name, cwd) in [
            (true, "cmd.exe", r"C:\Users\me"),
            (true, "bash.exe", "D:/code"),
            (true, "ssh.exe", r"C:\"),
            (false, "bash", "/home/me"),
            (false, "vim", "/srv/a b"),
        ] {
            let found = folder(windows, [front(name, cwd), elsewhere()]);
            assert_eq!(found.as_deref(), Some(cwd), "{name}");
        }
        // Nothing is known, or the process in front cannot be read.
        assert_eq!(folder(true, []), None);
        assert_eq!(folder(true, [elsewhere()]), None);
        let unread = json!({ "foreground": { "pid": 7, "name": "sudo" } });
        for payload in [unread, json!({ "foreground": null }), json!({})] {
            let told = Told::read("process_changed", &payload).unwrap();
            assert_eq!(folder(false, [front("bash", "/a"), told]), None);
        }
        assert_eq!(Told::read("bell", &json!({})), None);
    }

    #[test]
    fn of_a_powershell_only_what_it_printed_itself_counts() {
        let shell = || front("powershell.exe", r"C:\Users\me");
        assert_eq!(folder(true, [shell()]), None);
        for (text, found) in [
            (r"C:\Windows", Some(r"C:\Windows")),
            ("file://box/C:/Windows/a%20b", Some(r"C:\Windows\a b")),
            ("FILE:///d:/code", Some(r"D:\code")),
            (r"\\server\share", None),
            ("file://box/home/me", None),
            ("file://box/C:/%zz", None),
            ("Windows", None),
            ("", None),
        ] {
            let told = [shell(), printed(text, 7, "powershell.exe")];
            assert_eq!(folder(true, told).as_deref(), found, "{text}");
        }
        // Printed by another program, or by one that nothing is told of.
        for (pid, name) in [(8, "powershell.exe"), (7, "ssh.exe")] {
            let told = [shell(), printed(r"C:\Windows", pid, name)];
            assert_eq!(folder(true, told), None, "{pid} {name}");
        }
        for owner in [json!(null), json!({ "pid": 7 }), json!("powershell.exe")] {
            let payload = json!({ "pwd": r"C:\Windows", "owner": owner });
            let told = Told::read("pwd_changed", &payload).unwrap();
            assert_eq!(folder(true, [shell(), told]), None);
        }
        // While another program is in front, its own folder is the one.
        let told = [
            shell(),
            printed(r"C:\Windows", 7, "powershell.exe"),
            front("git.exe", r"C:\code"),
        ];
        assert_eq!(folder(true, told).as_deref(), Some(r"C:\code"));
        for name in ["pwsh", "pwsh.exe", "PowerShell.EXE", "pwsh-preview"] {
            let told = [front(name, "/home/me"), printed("/srv", 7, name)];
            assert_eq!(folder(false, told).as_deref(), Some("/srv"), "{name}");
        }
    }

    #[test]
    fn a_path_is_taken_only_as_the_machine_spells_it_and_with_nothing_unseen_in_it() {
        let long = format!("/{}", "x".repeat(TEXT_MAX_BYTES));
        for (windows, path) in [
            (true, "/home/me"),
            (true, r"\\server\share"),
            (true, r"\\?\C:\x"),
            (true, "C:"),
            (true, "C:x"),
            (true, r"1:\x"),
            (true, ""),
            (false, r"C:\Users"),
            (false, "home"),
            (false, "~/code"),
            (false, "/a\u{202e}b"),
            (false, "/a\nb"),
            (false, "/a\u{0}b"),
            (false, &long),
        ] {
            assert_eq!(folder(windows, [front("sh", path)]), None, "{path:?}");
        }
    }

    #[test]
    fn what_was_told_earlier_takes_the_place_of_nothing() {
        let mut whereabouts = Whereabouts::default();
        whereabouts.note(5, front("bash", "/new"));
        whereabouts.note(4, front("bash", "/old"));
        assert_eq!(whereabouts.folder(false).as_deref(), Some("/new"));
        whereabouts.note(5, front("pwsh", "/new"));
        whereabouts.note(9, printed("/printed", 7, "pwsh"));
        whereabouts.note(8, printed("/before", 7, "pwsh"));
        assert_eq!(whereabouts.folder(false).as_deref(), Some("/printed"));
    }

    #[test]
    fn what_is_kept_is_short() {
        let long = "x".repeat(FOLDER_PRINTED_MAX_BYTES + 1);
        assert_eq!(
            printed(&long, 7, "pwsh"),
            Told::Printed(None),
            "a printed folder"
        );
        assert_eq!(printed("/a", 7, &long), Told::Printed(None));
        assert_eq!(front(&long, "/a"), Told::Front(None));
        assert_eq!(folder(false, [front("sh", &long)]), None);
        // Printed, a folder may be longer than it may be taken.
        let printed_long = format!("/{}", "x".repeat(TEXT_MAX_BYTES));
        let told = [front("pwsh", "/"), printed(&printed_long, 7, "pwsh")];
        assert_eq!(folder(false, told), None);
    }
}
