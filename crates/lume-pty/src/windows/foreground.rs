//! Which program has a pseudoconsole.
//!
//! Windows keeps no record of which program a console's input goes to. What it does
//! keep is who started whom, so the answer is found by walking down from the program
//! that was started in the pseudoconsole: a shell that runs an editor has the editor as
//! its child, and the editor is the one reading the keyboard.
//!
//! The record of who started whom names a parent by its process id alone, and Windows
//! hands the ids of ended processes out again. A process is therefore only taken to be
//! a child if it was started no earlier than its supposed parent.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
};

use crate::{ForegroundProbe, ForegroundProcess};

/// Programs that serve a terminal session rather than run in one: the console hosts,
/// and the process WSL keeps beside every shell it starts in a distribution. They are
/// never the answer.
const HELPERS: [&str; 3] = ["conhost", "OpenConsole", "wslhost"];

/// How far down from the terminal's program the search goes. Real chains are a handful
/// of programs long; the limit only keeps a malformed process list from being walked
/// for ever.
const DESCENT_MAX: usize = 32;

/// How long a list of the system's processes is used again before it is read afresh.
/// Short enough that a program started a moment ago is found at the next question or
/// the one after, and long enough that the terminals of one window, asked about one
/// after the other, are answered from one reading.
const PROCESS_LIST_REUSE: Duration = Duration::from_millis(100);

/// When a process was started, in the system's own unit. Later is larger.
type Started = u64;

/// One process of the system's list.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    pid: u32,
    parent: u32,
    /// The program's file name as the system lists it, with its extension.
    file: String,
}

/// A program's name as it is shown: its file name without the folder and without a
/// trailing `.exe`. `None` if nothing is left.
fn program_name(file: &str) -> Option<String> {
    let file = file.rsplit(['\\', '/']).next().unwrap_or(file);
    let stem = match file.len().checked_sub(4) {
        Some(at) if file.is_char_boundary(at) && file[at..].eq_ignore_ascii_case(".exe") => {
            &file[..at]
        }
        _ => file,
    };
    (!stem.is_empty()).then(|| stem.to_owned())
}

fn is_helper(file: &str) -> bool {
    program_name(file).is_some_and(|name| {
        HELPERS
            .iter()
            .any(|helper| helper.eq_ignore_ascii_case(&name))
    })
}

/// Walks down from the process `root`, at each step to the child that was started
/// last, and returns where the walk ends: the program that most likely has the
/// terminal. `None` if `root` is not in the list.
///
/// `started` says when a process was started, or `None` if the system would not say.
/// A process whose start is unknown may be walked to, since its entry in the list is
/// its own, but not past: whether the processes that name it as their parent are
/// really its children cannot be checked.
fn deepest<'a>(
    entries: &'a [Entry],
    root: u32,
    root_started: Option<Started>,
    started: &mut dyn FnMut(u32) -> Option<Started>,
) -> Option<&'a Entry> {
    let mut at = entries.iter().find(|entry| entry.pid == root)?;
    let mut at_started = root_started;
    let mut seen = vec![root];
    for _ in 0..DESCENT_MAX {
        let Some(parent_started) = at_started else {
            break;
        };
        let mut newest: Option<(&Entry, Option<Started>)> = None;
        for child in entries {
            if child.parent != at.pid || seen.contains(&child.pid) || is_helper(&child.file) {
                continue;
            }
            let child_started = started(child.pid);
            if child_started.is_some_and(|child_started| child_started < parent_started) {
                // Older than the process it names as its parent: that parent ended
                // long ago and its id now belongs to this one.
                continue;
            }
            // A child whose start is unknown ranks behind every child whose start is
            // known; among equals the later entry of the list wins.
            if newest.is_none_or(|(_, newest_started)| child_started >= newest_started) {
                newest = Some((child, child_started));
            }
        }
        let Some((child, child_started)) = newest else {
            break;
        };
        seen.push(child.pid);
        at = child;
        at_started = child_started;
    }
    Some(at)
}

fn filetime(time: FILETIME) -> Started {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// When the process behind `handle` was started.
fn started_of(handle: HANDLE) -> Option<Started> {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut creation, mut exit, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the caller's handle is open, and the four out pointers are valid.
    let ok = unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    (ok != 0).then(|| filetime(creation))
}

/// When the process with this id was started, if the system lets this process ask.
/// It does not for the programs of other users and for protected ones.
fn started_of_pid(pid: u32) -> Option<Started> {
    // SAFETY: takes no pointers. The least access that lets the times be read.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: the call succeeded, so this is an open handle that nothing else owns.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    started_of(handle.as_raw_handle())
}

/// The list of processes last read, and when reading it began.
static LISTED: Mutex<Option<(Instant, Arc<[Entry]>)>> = Mutex::new(None);

/// Every process there is, as of now or of a moment ago.
///
/// Reading the list is the slow part of a question, a few milliseconds on a machine
/// with a few hundred processes, and it is the same list whichever terminal is asked
/// about. So terminals asked about at about the same time share one reading.
fn processes() -> Option<Arc<[Entry]>> {
    let listed = || LISTED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((read, list)) = &*listed()
        && read.elapsed() < PROCESS_LIST_REUSE
    {
        return Some(Arc::clone(list));
    }
    // Read with the lock let go: two questions that both find the list too old both
    // read it, which costs time once and is otherwise harmless.
    let reading = Instant::now();
    let list: Arc<[Entry]> = read_processes()?.into();
    *listed() = Some((reading, Arc::clone(&list)));
    Some(list)
}

/// Every process there is right now.
fn read_processes() -> Option<Vec<Entry>> {
    // SAFETY: takes no pointers.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    // SAFETY: the call succeeded, so this is an open handle that nothing else owns.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };

    // SAFETY: plain data for which all-zero is a valid value.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut entries = Vec::new();
    // SAFETY: the snapshot is open and `entry` has its size set, as the call requires.
    let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) };
    while more != 0 {
        let name = &entry.szExeFile;
        let len = name
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(name.len());
        entries.push(Entry {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            file: String::from_utf16_lossy(&name[..len]),
        });
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) };
    }
    Some(entries)
}

/// Asks which program has the pseudoconsole that `process` was started in.
pub(crate) struct WinForeground {
    /// The program that was started in the pseudoconsole. While this handle is open
    /// its process id is not given to another process.
    process: Arc<OwnedHandle>,
    pid: u32,
}

impl WinForeground {
    pub(crate) fn new(process: Arc<OwnedHandle>, pid: u32) -> Self {
        Self { process, pid }
    }
}

impl ForegroundProbe for WinForeground {
    fn foreground(&self) -> Option<ForegroundProcess> {
        let process = self.process.as_raw_handle();
        // SAFETY: the handle is open for as long as `self.process` is alive. A zero
        // timeout only asks whether the process has ended.
        if unsafe { WaitForSingleObject(process, 0) } == WAIT_OBJECT_0 {
            return None;
        }
        let entries = processes()?;
        let found = deepest(&entries, self.pid, started_of(process), &mut started_of_pid)?;
        Some(ForegroundProcess {
            pid: found.pid,
            name: program_name(&found.file)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32, parent: u32, file: &str) -> Entry {
        Entry {
            pid,
            parent,
            file: file.to_owned(),
        }
    }

    /// Where the walk from process 10 ends, with each process started at the time the
    /// table gives for it.
    fn found(entries: &[Entry], times: &[(u32, Started)]) -> Option<u32> {
        let time = |pid: u32| times.iter().find(|(of, _)| *of == pid).map(|(_, at)| *at);
        deepest(entries, 10, time(10), &mut |pid| time(pid)).map(|entry| entry.pid)
    }

    #[test]
    fn a_name_is_shown_without_folder_and_without_its_exe() {
        assert_eq!(program_name("cmd.exe").as_deref(), Some("cmd"));
        assert_eq!(program_name("PING.EXE").as_deref(), Some("PING"));
        assert_eq!(program_name(r"C:\tools\vim.exe").as_deref(), Some("vim"));
        assert_eq!(program_name("node").as_deref(), Some("node"));
        assert_eq!(
            program_name("archive.tar.exe").as_deref(),
            Some("archive.tar")
        );
        assert_eq!(program_name("exe").as_deref(), Some("exe"));
        assert_eq!(program_name("é.exe").as_deref(), Some("é"));
        assert_eq!(program_name("éé").as_deref(), Some("éé"));
        assert_eq!(program_name(".exe"), None);
        assert_eq!(program_name(""), None);
    }

    #[test]
    fn a_shell_with_nothing_under_it_is_the_answer_itself() {
        let entries = [entry(1, 0, "System"), entry(10, 1, "cmd.exe")];
        assert_eq!(found(&entries, &[(10, 100)]), Some(10));
    }

    #[test]
    fn a_process_that_is_not_listed_has_no_answer() {
        let entries = [entry(11, 10, "ping.exe")];
        assert_eq!(found(&entries, &[(10, 100), (11, 200)]), None);
    }

    #[test]
    fn the_walk_goes_down_to_the_last_program_of_the_chain() {
        let entries = [
            entry(10, 1, "pwsh.exe"),
            entry(11, 10, "git.exe"),
            entry(12, 11, "less.exe"),
            entry(13, 99, "unrelated.exe"),
        ];
        let times = [(10, 100), (11, 200), (12, 300), (13, 400)];
        assert_eq!(found(&entries, &times), Some(12));
    }

    #[test]
    fn of_several_children_the_one_started_last_is_followed() {
        let entries = [
            entry(10, 1, "cmd.exe"),
            entry(11, 10, "server.exe"),
            entry(12, 10, "vim.exe"),
            entry(13, 10, "watcher.exe"),
            entry(14, 11, "worker.exe"),
        ];
        let times = [(10, 100), (11, 200), (12, 500), (13, 300), (14, 900)];
        assert_eq!(found(&entries, &times), Some(12));
    }

    #[test]
    fn helpers_are_passed_over_however_they_are_spelt() {
        let entries = [
            entry(10, 1, "cmd.exe"),
            entry(11, 10, "ping.exe"),
            entry(12, 10, "conhost.exe"),
            entry(13, 10, "OPENCONSOLE.EXE"),
        ];
        let times = [(10, 100), (11, 200), (12, 300), (13, 400)];
        assert_eq!(found(&entries, &times), Some(11));

        // With only helpers under it, the shell is the answer.
        assert_eq!(found(&entries[..1], &times), Some(10));
        let only_helpers = [entries[0].clone(), entries[2].clone()];
        assert_eq!(found(&only_helpers, &times), Some(10));

        // A distribution's pane ends at the program that reaches into the
        // distribution, not at the process that sits beside it.
        let distribution = [
            entry(10, 1, "wsl.exe"),
            entry(11, 10, "wsl.exe"),
            entry(12, 11, "wslhost.exe"),
        ];
        assert_eq!(
            found(&distribution, &[(10, 100), (11, 200), (12, 300)]),
            Some(11)
        );
    }

    #[test]
    fn a_process_older_than_its_supposed_parent_is_not_its_child() {
        // Process 11 names 10 as its parent, but was started before 10 was: the 10
        // that started it has ended, and its id was given to the shell.
        let entries = [entry(10, 1, "cmd.exe"), entry(11, 10, "old.exe")];
        assert_eq!(found(&entries, &[(10, 500), (11, 100)]), Some(10));
        // Started at the same instant is not older.
        assert_eq!(found(&entries, &[(10, 500), (11, 500)]), Some(11));
    }

    #[test]
    fn a_process_that_cannot_be_asked_is_walked_to_and_not_past() {
        let entries = [
            entry(10, 1, "cmd.exe"),
            entry(11, 10, "elevated.exe"),
            entry(12, 11, "below.exe"),
        ];
        // Nothing is known of 11, so it is accepted on the strength of its own entry,
        // and whatever names it as its parent is left alone.
        assert_eq!(found(&entries, &[(10, 100), (12, 300)]), Some(11));
        // A child whose start is known is preferred to one whose start is not.
        let entries = [
            entry(10, 1, "cmd.exe"),
            entry(11, 10, "known.exe"),
            entry(12, 10, "unknown.exe"),
        ];
        assert_eq!(found(&entries, &[(10, 100), (11, 200)]), Some(11));
        // Without the shell's own start, nothing under it can be checked.
        assert_eq!(found(&entries, &[(11, 200)]), Some(10));
    }

    #[test]
    fn a_list_that_loops_ends_the_walk() {
        // No real list says this, but the walk must end whatever it is given.
        let entries = [
            entry(10, 12, "a.exe"),
            entry(11, 10, "b.exe"),
            entry(12, 11, "c.exe"),
        ];
        let times = [(10, 100), (11, 100), (12, 100)];
        assert_eq!(found(&entries, &times), Some(12));

        let long: Vec<Entry> = (10..200).map(|pid| entry(pid, pid - 1, "x.exe")).collect();
        let times: Vec<(u32, Started)> = (10..200).map(|pid| (pid, u64::from(pid))).collect();
        assert_eq!(found(&long, &times), Some(10 + DESCENT_MAX as u32));
    }
}
