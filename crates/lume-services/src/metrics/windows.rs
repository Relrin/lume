//! Reading a Windows system: the processor's times, the memory status, the counters of
//! the network adapters that are hardware, and the graphics card's statistics from the
//! display kernel (the same ones Task Manager shows).

use std::collections::HashMap;
use std::time::Instant;

use windows_sys::Wdk::Graphics::Direct3D::{
    D3DKMT_ADAPTERINFO, D3DKMT_CLOSEADAPTER, D3DKMT_ENUMADAPTERS2, D3DKMT_QUERYSTATISTICS,
    D3DKMT_QUERYSTATISTICS_ADAPTER, D3DKMT_QUERYSTATISTICS_NODE, D3DKMT_QUERYSTATISTICS_SEGMENT,
    D3DKMTCloseAdapter, D3DKMTEnumAdapters2, D3DKMTQueryStatistics,
};
use windows_sys::Win32::Foundation::{FILETIME, LUID};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_TABLE2,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;

use super::{Counters, CpuTimes, GpuMemory, GpuSource, System, Usage};

pub struct WindowsSystem;

fn ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// The flags of an adapter that say it is hardware, and that it is a filter set over
/// another one (which would count its traffic twice).
const HARDWARE_INTERFACE: u8 = 0x01;
const FILTER_INTERFACE: u8 = 0x02;

impl System for WindowsSystem {
    fn cpu(&mut self) -> std::io::Result<CpuTimes> {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut idle, mut kernel, mut user) = (zero, zero, zero);
        // SAFETY: three out-parameters the call fills.
        let done = unsafe { GetSystemTimes(&raw mut idle, &raw mut kernel, &raw mut user) };
        if done == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // The kernel's time includes the idle time.
        Ok(CpuTimes {
            idle: ticks(idle),
            total: ticks(kernel) + ticks(user),
        })
    }

    fn memory(&mut self) -> std::io::Result<Usage> {
        // SAFETY: a plain structure the call fills, its length set as it asks.
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).unwrap_or(0);
        // SAFETY: as above.
        if unsafe { GlobalMemoryStatusEx(&raw mut status) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Usage {
            used: status.ullTotalPhys.saturating_sub(status.ullAvailPhys),
            total: status.ullTotalPhys,
        })
    }

    fn network(&mut self) -> std::io::Result<Counters> {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        // SAFETY: the call allocates the table and gives it back to be freed below.
        let status = unsafe { GetIfTable2(&raw mut table) };
        if status != 0 || table.is_null() {
            return Err(std::io::Error::from_raw_os_error(
                i32::try_from(status).unwrap_or(0),
            ));
        }
        let mut counters = Counters::default();
        // SAFETY: the table holds `NumEntries` rows from its `Table` field on.
        unsafe {
            let count = (*table).NumEntries as usize;
            let rows = std::ptr::addr_of!((*table).Table)
                .cast::<windows_sys::Win32::NetworkManagement::IpHelper::MIB_IF_ROW2>();
            for index in 0..count {
                let row = &*rows.add(index);
                let flags = row.InterfaceAndOperStatusFlags._bitfield;
                let counted = row.Type != IF_TYPE_SOFTWARE_LOOPBACK
                    && row.OperStatus == IfOperStatusUp
                    && flags & HARDWARE_INTERFACE != 0
                    && flags & FILTER_INTERFACE == 0;
                if counted {
                    counters.received += row.InOctets;
                    counters.sent += row.OutOctets;
                }
            }
            FreeMibTable(table.cast());
        }
        Ok(counters)
    }
}

/// The most adapters looked at.
const ADAPTERS_MAX: usize = 16;

/// The graphics adapters, as the display kernel lists them.
fn adapters() -> Vec<D3DKMT_ADAPTERINFO> {
    // SAFETY: a plain structure, filled by the calls below.
    let mut infos: Vec<D3DKMT_ADAPTERINFO> = vec![unsafe { std::mem::zeroed() }; ADAPTERS_MAX];
    let mut list = D3DKMT_ENUMADAPTERS2 {
        NumAdapters: u32::try_from(ADAPTERS_MAX).unwrap_or(0),
        pAdapters: infos.as_mut_ptr(),
    };
    // SAFETY: the list names a buffer of `NumAdapters` entries.
    if unsafe { D3DKMTEnumAdapters2(&raw mut list) } != 0 {
        return Vec::new();
    }
    infos.truncate(list.NumAdapters as usize);
    infos
}

fn close(adapters: &[D3DKMT_ADAPTERINFO]) {
    for adapter in adapters {
        let close = D3DKMT_CLOSEADAPTER {
            hAdapter: adapter.hAdapter,
        };
        // SAFETY: a handle the listing opened.
        unsafe {
            D3DKMTCloseAdapter(&raw const close);
        }
    }
}

/// One question to the display kernel about an adapter.
fn query(
    luid: LUID,
    kind: i32,
    set: impl FnOnce(&mut D3DKMT_QUERYSTATISTICS),
) -> Option<D3DKMT_QUERYSTATISTICS> {
    // SAFETY: a plain structure the call fills.
    let mut statistics: D3DKMT_QUERYSTATISTICS = unsafe { std::mem::zeroed() };
    statistics.Type = kind;
    statistics.AdapterLuid = luid;
    set(&mut statistics);
    // SAFETY: as above.
    (unsafe { D3DKMTQueryStatistics(&raw const statistics) } == 0).then_some(statistics)
}

/// What one adapter is made of.
struct Adapter {
    luid: LUID,
    nodes: u32,
    segments: u32,
    /// The memory of its own it has: the limits of its segments that are not windows
    /// onto the system's memory.
    dedicated: u64,
}

fn describe(luid: LUID) -> Option<Adapter> {
    let statistics = query(luid, D3DKMT_QUERYSTATISTICS_ADAPTER, |_| {})?;
    // SAFETY: the union holds the adapter's information for this kind of question.
    let information = unsafe { statistics.QueryResult.AdapterInformation };
    let mut adapter = Adapter {
        luid,
        nodes: information.NodeCount,
        segments: information.NbSegments,
        dedicated: 0,
    };
    for (_, limit, aperture) in segments(&adapter) {
        if !aperture {
            adapter.dedicated += limit;
        }
    }
    Some(adapter)
}

/// Each segment's bytes committed and limit, and whether it is a window onto the
/// system's memory.
fn segments(adapter: &Adapter) -> Vec<(u64, u64, bool)> {
    (0..adapter.segments)
        .filter_map(|segment| {
            let statistics = query(adapter.luid, D3DKMT_QUERYSTATISTICS_SEGMENT, |s| {
                s.Anonymous.QuerySegment.SegmentId = segment;
            })?;
            // SAFETY: the union holds the segment's information for this question.
            let information = unsafe { statistics.QueryResult.SegmentInformation };
            Some((
                information.BytesCommitted,
                information.CommitLimit,
                information.Aperture != 0,
            ))
        })
        .collect()
}

/// The running time of each of an adapter's engines, in hundreds of nanoseconds.
fn running_times(adapter: &Adapter) -> HashMap<u32, i64> {
    (0..adapter.nodes)
        .filter_map(|node| {
            let statistics = query(adapter.luid, D3DKMT_QUERYSTATISTICS_NODE, |s| {
                s.Anonymous.QueryNode.NodeId = node;
            })?;
            // SAFETY: the union holds the node's information for this question.
            let information = unsafe { statistics.QueryResult.NodeInformation };
            Some((node, information.GlobalInformation.RunningTime))
        })
        .collect()
}

/// The adapter Lume reports: the one with the most memory of its own, which is the
/// discrete card where there is one; else the one with the most engines.
struct Card {
    adapter: Adapter,
    before: Option<(Instant, HashMap<u32, i64>)>,
}

pub fn open_gpu() -> Option<Box<dyn GpuSource>> {
    let listed = adapters();
    let mut found: Vec<Adapter> = listed
        .iter()
        .filter_map(|info| describe(info.AdapterLuid))
        .filter(|adapter| adapter.nodes > 0)
        .collect();
    close(&listed);
    found.sort_by_key(|adapter| (adapter.dedicated, adapter.nodes));
    let adapter = found.pop()?;
    Some(Box::new(Card {
        adapter,
        before: None,
    }))
}

impl GpuSource for Card {
    fn read(&mut self) -> std::io::Result<(Option<f32>, Option<GpuMemory>)> {
        let now = Instant::now();
        let times = running_times(&self.adapter);
        if times.is_empty() {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        let busy = self.before.as_ref().and_then(|(then, before)| {
            #[allow(clippy::cast_precision_loss, reason = "a share")]
            let wall = now.duration_since(*then).as_nanos() as f64 / 100.0;
            if wall <= 0.0 {
                return None;
            }
            times
                .iter()
                .filter_map(|(node, time)| {
                    let ran = time.checked_sub(*before.get(node)?)?;
                    #[allow(clippy::cast_precision_loss, reason = "a share")]
                    Some((ran.max(0) as f64 / wall).min(1.0))
                })
                .reduce(f64::max)
        });
        self.before = Some((now, times));
        let memory = (self.adapter.dedicated > 0).then(|| {
            let used = segments(&self.adapter)
                .into_iter()
                .filter(|(_, _, aperture)| !aperture)
                .map(|(committed, _, _)| committed)
                .sum();
            GpuMemory {
                used,
                total: Some(self.adapter.dedicated),
            }
        });
        #[allow(clippy::cast_possible_truncation, reason = "a share between 0 and 1")]
        Ok((busy.map(|busy| busy as f32), memory))
    }
}
