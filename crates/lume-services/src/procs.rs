//! What a program and everything it started use: how many processes, how much memory,
//! how much processor time.
//!
//! The tree is read from what the system says about every process: on Windows a
//! snapshot of them all, on Linux `/proc`, on macOS the kernel's list of each process's
//! children. Memory is the resident set (the working set on Windows), added up over the
//! tree, so memory shared between processes counts once for each.

use std::time::Duration;

/// What a process and all its descendants use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeUsage {
    /// How many processes the tree has, the first one included.
    pub processes: u32,
    pub memory_bytes: u64,
    /// The processor time they have used, in the system and outside it.
    pub cpu_time: Duration,
}

/// What a process and all its descendants use, or `None` when there is no such process.
pub fn tree_usage(root_pid: u32) -> Option<TreeUsage> {
    system::tree_usage(root_pid)
}

/// One process, as the tree is worked out from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(any(test, windows, target_os = "linux")),
    allow(
        dead_code,
        reason = "the tree is worked out this way on Windows and Linux"
    )
)]
struct Process {
    pid: u32,
    parent: u32,
    /// When it started, in any one unit; a process is never the child of one that started
    /// after it, whatever its parent's id says (an id can be used again).
    started: u64,
}

/// The processes of the tree under `root`, `root` first.
#[cfg_attr(
    not(any(test, windows, target_os = "linux")),
    allow(
        dead_code,
        reason = "the tree is worked out this way on Windows and Linux"
    )
)]
fn tree(processes: &[Process], root: u32) -> Vec<u32> {
    let Some(first) = processes.iter().find(|process| process.pid == root) else {
        return Vec::new();
    };
    let mut found = vec![*first];
    let mut next = 0;
    while next < found.len() {
        let parent = found[next];
        next += 1;
        for process in processes {
            if process.parent == parent.pid
                && process.pid != parent.pid
                && process.started >= parent.started
                && !found.iter().any(|seen| seen.pid == process.pid)
            {
                found.push(*process);
            }
        }
    }
    found.into_iter().map(|process| process.pid).collect()
}

/// The fields of `/proc/<pid>/stat` a tree needs: the parent, the times in clock ticks
/// (user, system, start) and the resident pages.
#[cfg_attr(
    not(any(test, target_os = "linux")),
    allow(dead_code, reason = "read on Linux only")
)]
fn parse_stat(text: &str) -> Option<(u32, u64, u64, u64, u64)> {
    // The program's name is in brackets and may hold anything, brackets too.
    let rest = &text[text.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After the name: state (3), parent (4), ..., user (14), system (15), ..., start
    // (22), virtual size (23), resident pages (24), counted from 1 over the whole line.
    let field = |number: usize| fields.get(number - 3).and_then(|value| value.parse().ok());
    Some((
        u32::try_from(field(4)?).ok()?,
        field(14)?,
        field(15)?,
        field(22)?,
        field(24)?,
    ))
}

#[cfg(windows)]
mod system {
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };

    use super::{Process, TreeUsage, tree};

    fn ticks(time: FILETIME) -> u64 {
        (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
    }

    /// Every process the system has, with its parent.
    fn snapshot() -> Vec<(u32, u32)> {
        // SAFETY: a snapshot of the processes, closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut all = Vec::new();
        // SAFETY: a plain structure the calls fill, its size set as they ask.
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap_or(0);
        // SAFETY: the snapshot above and the structure.
        let mut more = unsafe { Process32FirstW(snapshot, &raw mut entry) } != 0;
        while more {
            all.push((entry.th32ProcessID, entry.th32ParentProcessID));
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot, &raw mut entry) } != 0;
        }
        // SAFETY: the snapshot, closed once.
        unsafe { CloseHandle(snapshot) };
        all
    }

    /// When a process started, and the processor time and memory it uses.
    fn read(pid: u32) -> Option<(u64, Duration, u64)> {
        // SAFETY: asks for a handle that can only be read; closed below.
        let mut handle =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid) };
        if handle.is_null() {
            // SAFETY: as above, asking for less.
            handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        }
        if handle.is_null() {
            return None;
        }
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: the handle above and four structures to fill.
        let timed = unsafe {
            GetProcessTimes(
                handle,
                &raw mut created,
                &raw mut exited,
                &raw mut kernel,
                &raw mut user,
            )
        } != 0;
        // SAFETY: a plain structure the call fills, its size given.
        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        let size = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).unwrap_or(0);
        counters.cb = size;
        // SAFETY: the handle above and the structure.
        let measured = unsafe { GetProcessMemoryInfo(handle, &raw mut counters, size) } != 0;
        // SAFETY: the handle opened above, closed once.
        unsafe { CloseHandle(handle) };
        if !timed {
            return None;
        }
        // Processor time is counted in hundreds of nanoseconds.
        let cpu = Duration::from_nanos((ticks(kernel) + ticks(user)).saturating_mul(100));
        let memory = if measured {
            counters.WorkingSetSize as u64
        } else {
            0
        };
        Some((ticks(created), cpu, memory))
    }

    pub fn tree_usage(root: u32) -> Option<TreeUsage> {
        // The tree by parents' ids alone first, so only its processes are opened.
        let all: Vec<Process> = snapshot()
            .into_iter()
            .map(|(pid, parent)| Process {
                pid,
                parent,
                started: 0,
            })
            .collect();
        let candidates = tree(&all, root);
        // Only the processes that can be read are part of the tree; their start times
        // tell a real child from one whose parent's id was used again.
        let mut readings = Vec::new();
        let mut processes = Vec::new();
        for candidate in all
            .iter()
            .filter(|process| candidates.contains(&process.pid))
        {
            let (pid, parent) = (candidate.pid, candidate.parent);
            if let Some((started, cpu, memory)) = read(pid) {
                processes.push(Process {
                    pid,
                    parent,
                    started,
                });
                readings.push((pid, cpu, memory));
            }
        }
        let members = tree(&processes, root);
        if members.is_empty() {
            return None;
        }
        let mut usage = TreeUsage::default();
        for (pid, cpu, memory) in readings {
            if members.contains(&pid) {
                usage.processes += 1;
                usage.memory_bytes += memory;
                usage.cpu_time += cpu;
            }
        }
        Some(usage)
    }
}

#[cfg(target_os = "linux")]
mod system {
    use std::time::Duration;

    use super::{Process, TreeUsage, parse_stat, tree};

    pub fn tree_usage(root: u32) -> Option<TreeUsage> {
        // SAFETY: no arguments.
        let ticks = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) })
            .ok()
            .filter(|ticks| *ticks > 0)
            .unwrap_or(100);
        // SAFETY: no arguments.
        let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(4096);
        let mut processes = Vec::new();
        let mut readings = Vec::new();
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse().ok())
            else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            let Some((parent, user, system, started, resident)) = parse_stat(&text) else {
                continue;
            };
            processes.push(Process {
                pid,
                parent,
                started,
            });
            readings.push((pid, user + system, resident));
        }
        let members = tree(&processes, root);
        if members.is_empty() {
            return None;
        }
        let mut usage = TreeUsage::default();
        for (pid, cpu, resident) in readings {
            if members.contains(&pid) {
                usage.processes += 1;
                usage.memory_bytes += resident * page;
                usage.cpu_time += Duration::from_nanos(cpu * 1_000_000_000 / ticks);
            }
        }
        Some(usage)
    }
}

#[cfg(target_os = "macos")]
mod system {
    use std::ffi::c_int;
    use std::time::Duration;

    use super::TreeUsage;

    /// Asks `proc_listpids` for the processes whose parent is the one given, as
    /// `sys/proc_info.h` numbers it.
    const PROC_PPID_ONLY: u32 = 6;

    /// How the machine's processor ticks turn into nanoseconds: times `numer`, over
    /// `denom`.
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }

    unsafe extern "C" {
        fn mach_timebase_info(info: *mut Timebase) -> c_int;
    }

    /// The children of a process, as the kernel lists them.
    fn children(pid: libc::pid_t) -> Vec<libc::pid_t> {
        let mut capacity = 64usize;
        loop {
            let mut buffer = vec![0 as libc::pid_t; capacity];
            let bytes = c_int::try_from(capacity * std::mem::size_of::<libc::pid_t>())
                .unwrap_or(c_int::MAX);
            // SAFETY: a buffer of the size given, which the call fills with ids.
            let filled = unsafe {
                libc::proc_listpids(
                    PROC_PPID_ONLY,
                    u32::try_from(pid).unwrap_or(0),
                    buffer.as_mut_ptr().cast(),
                    bytes,
                )
            };
            let Ok(filled) = usize::try_from(filled) else {
                return Vec::new();
            };
            let count = filled / std::mem::size_of::<libc::pid_t>();
            if count < capacity {
                buffer.truncate(count);
                buffer.retain(|child| *child > 0);
                return buffer;
            }
            capacity *= 4;
        }
    }

    /// The processor time and memory of one process.
    fn read(pid: libc::pid_t) -> Option<(u64, u64)> {
        // SAFETY: a plain structure the call fills.
        let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
        let size = c_int::try_from(std::mem::size_of::<libc::proc_taskinfo>()).ok()?;
        // SAFETY: the structure and its size.
        let filled = unsafe {
            libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, (&raw mut info).cast(), size)
        };
        (filled == size).then_some((
            info.pti_total_user + info.pti_total_system,
            info.pti_resident_size,
        ))
    }

    pub fn tree_usage(root: u32) -> Option<TreeUsage> {
        let root = libc::pid_t::try_from(root).ok()?;
        read(root)?;
        // Processor time is counted in the machine's own ticks.
        let mut timebase = Timebase { numer: 1, denom: 1 };
        // SAFETY: a structure the call fills.
        unsafe { mach_timebase_info(&raw mut timebase) };
        let (numer, denom) = (u64::from(timebase.numer), u64::from(timebase.denom.max(1)));
        let mut usage = TreeUsage::default();
        let mut pending = vec![root];
        let mut seen = Vec::new();
        while let Some(pid) = pending.pop() {
            if seen.contains(&pid) {
                continue;
            }
            seen.push(pid);
            let Some((ticks, memory)) = read(pid) else {
                continue;
            };
            usage.processes += 1;
            usage.memory_bytes += memory;
            usage.cpu_time += Duration::from_nanos(
                u64::try_from(u128::from(ticks) * u128::from(numer) / u128::from(denom))
                    .unwrap_or(u64::MAX),
            );
            pending.extend(children(pid));
        }
        Some(usage)
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod system {
    use super::TreeUsage;

    pub fn tree_usage(_root: u32) -> Option<TreeUsage> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, parent: u32, started: u64) -> Process {
        Process {
            pid,
            parent,
            started,
        }
    }

    #[test]
    fn the_tree_holds_every_descendant_once() {
        let processes = [
            process(1, 0, 0),
            process(10, 1, 5),
            process(11, 10, 6),
            process(12, 10, 7),
            process(13, 11, 8),
            process(20, 1, 9),
        ];
        let mut found = tree(&processes, 10);
        assert_eq!(found[0], 10);
        found.sort_unstable();
        assert_eq!(found, [10, 11, 12, 13]);
        assert!(tree(&processes, 99).is_empty());
    }

    #[test]
    fn a_process_older_than_its_parent_is_not_its_child() {
        // 30 started before 10 did: its parent was an earlier process with that id.
        let processes = [process(10, 1, 50), process(30, 10, 20), process(31, 10, 60)];
        assert_eq!(tree(&processes, 10), [10, 31]);
    }

    #[test]
    fn a_process_that_names_itself_as_parent_is_counted_once() {
        let processes = [process(0, 0, 0), process(4, 0, 1)];
        assert_eq!(tree(&processes, 0), [0, 4]);
    }

    #[test]
    fn a_stat_line_is_read_past_a_name_with_brackets_and_spaces() {
        let text = "1234 (my (odd) prog) S 1000 1234 1234 0 -1 4194560 100 0 0 0 \
                    250 50 0 0 20 0 1 0 98765 1000000 321 18446744073709551615";
        assert_eq!(parse_stat(text), Some((1000, 250, 50, 98765, 321)));
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn this_process_is_a_tree_of_at_least_itself() {
        let usage = tree_usage(std::process::id());
        if cfg!(any(windows, target_os = "linux", target_os = "macos")) {
            let usage = usage.unwrap();
            assert!(usage.processes >= 1);
            assert!(usage.memory_bytes > 0);
        }
    }
}
