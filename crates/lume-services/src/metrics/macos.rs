//! Reading a Mac: the host's processor load and memory statistics, the counters of the
//! network interfaces, and the graphics card's performance statistics from the I/O
//! Kit's accelerator.

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};

use super::{Counters, CpuTimes, GpuMemory, GpuSource, System, Usage};

pub struct MacSystem {
    host: libc::mach_port_t,
    /// The counters of each interface, which are 32 bits and wrap; each is followed
    /// into a count that does not.
    seen: HashMap<String, (u32, u32, u64, u64)>,
}

impl MacSystem {
    pub fn new() -> Self {
        // SAFETY: the port of this task's host, kept for the life of the sampler.
        let host = unsafe { mach_host_self() };
        Self {
            host,
            seen: HashMap::new(),
        }
    }
}

fn failed(what: &str) -> std::io::Error {
    std::io::Error::other(format!("{what} could not be read"))
}

/// Interfaces that carry traffic the hardware ones carry as well, or none of a user's.
const VIRTUAL_PREFIXES: [&str; 9] = [
    "lo", "utun", "awdl", "llw", "bridge", "gif", "stf", "anpi", "ap",
];

impl System for MacSystem {
    fn cpu(&mut self) -> std::io::Result<CpuTimes> {
        let mut ticks = [0 as libc::integer_t; 4];
        let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
        // SAFETY: the buffer holds the four counts the flavour asks for.
        let status = unsafe {
            host_statistics(
                self.host,
                libc::HOST_CPU_LOAD_INFO,
                ticks.as_mut_ptr(),
                &raw mut count,
            )
        };
        if status != libc::KERN_SUCCESS {
            return Err(failed("the processor's load"));
        }
        // User, system, idle, nice.
        let ticks = ticks.map(|tick| u64::from(tick.cast_unsigned()));
        Ok(CpuTimes {
            idle: ticks[2],
            total: ticks.iter().sum(),
        })
    }

    fn memory(&mut self) -> std::io::Result<Usage> {
        let mut total: u64 = 0;
        let mut length = std::mem::size_of::<u64>();
        let name = c"hw.memsize";
        // SAFETY: a name and a buffer of the size given.
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&raw mut total).cast(),
                &raw mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: a plain structure the call fills, of the count it asks for.
        let mut statistics: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
        let mut count = libc::HOST_VM_INFO64_COUNT;
        // SAFETY: as above.
        let status = unsafe {
            host_statistics64(
                self.host,
                libc::HOST_VM_INFO64,
                (&raw mut statistics).cast(),
                &raw mut count,
            )
        };
        if status != libc::KERN_SUCCESS {
            return Err(failed("the memory"));
        }
        // SAFETY: no arguments.
        let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(4096);
        // What Activity Monitor calls memory used: what programs hold, what the system
        // holds for itself, and what is compressed.
        let pages = u64::from(statistics.internal_page_count)
            .saturating_sub(u64::from(statistics.purgeable_count))
            + u64::from(statistics.wire_count)
            + u64::from(statistics.compressor_page_count);
        Ok(Usage {
            used: (pages * page).min(total),
            total,
        })
    }

    fn network(&mut self) -> std::io::Result<Counters> {
        let mut first: *mut libc::ifaddrs = std::ptr::null_mut();
        // SAFETY: the call allocates the list, freed below.
        if unsafe { libc::getifaddrs(&raw mut first) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut counters = Counters::default();
        let mut entry = first;
        while !entry.is_null() {
            // SAFETY: an entry of the list the call gave.
            let here = unsafe { &*entry };
            entry = here.ifa_next;
            let address = here.ifa_addr;
            if address.is_null() || here.ifa_data.is_null() {
                continue;
            }
            // SAFETY: the address is a socket address of some family.
            if i32::from(unsafe { (*address).sa_family }) != libc::AF_LINK {
                continue;
            }
            let flags = here.ifa_flags;
            if flags & libc::IFF_LOOPBACK as u32 != 0 || flags & libc::IFF_UP as u32 == 0 {
                continue;
            }
            // SAFETY: the name is a C string of the list.
            let name = unsafe { std::ffi::CStr::from_ptr(here.ifa_name) }
                .to_string_lossy()
                .into_owned();
            if VIRTUAL_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                continue;
            }
            // SAFETY: for a link address the data is the interface's `if_data`.
            let data = unsafe { &*here.ifa_data.cast::<libc::if_data>() };
            let (received, sent) = (data.ifi_ibytes, data.ifi_obytes);
            let followed = self.seen.entry(name).or_insert((
                received,
                sent,
                u64::from(received),
                u64::from(sent),
            ));
            followed.2 += u64::from(received.wrapping_sub(followed.0));
            followed.3 += u64::from(sent.wrapping_sub(followed.1));
            followed.0 = received;
            followed.1 = sent;
            counters.received += followed.2;
            counters.sent += followed.3;
        }
        // SAFETY: the list the call gave.
        unsafe { libc::freeifaddrs(first) };
        Ok(counters)
    }
}

// The host's statistics, declared here as the system's headers declare them.
unsafe extern "C" {
    fn mach_host_self() -> libc::mach_port_t;
    fn host_statistics(
        host: libc::mach_port_t,
        flavor: i32,
        info: *mut libc::integer_t,
        count: *mut u32,
    ) -> i32;
    fn host_statistics64(
        host: libc::mach_port_t,
        flavor: i32,
        info: *mut libc::integer_t,
        count: *mut u32,
    ) -> i32;
}

// The I/O Kit and Core Foundation, as far as reading an accelerator's statistics goes.
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFMutableDictionaryRef = *mut c_void;
type IoObject = libc::mach_port_t;

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const CF_NUMBER_SINT64: isize = 4;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(
        allocator: CFTypeRef,
        text: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFDictionaryGetValue(dictionary: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, kind: isize, value: *mut c_void) -> u8;
    fn CFGetTypeID(object: CFTypeRef) -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFRelease(object: CFTypeRef);
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingServices(
        port: libc::mach_port_t,
        matching: CFDictionaryRef,
        iterator: *mut IoObject,
    ) -> i32;
    fn IOIteratorNext(iterator: IoObject) -> IoObject;
    fn IORegistryEntryCreateCFProperties(
        entry: IoObject,
        properties: *mut CFMutableDictionaryRef,
        allocator: CFTypeRef,
        options: u32,
    ) -> i32;
    fn IOObjectRelease(object: IoObject) -> i32;
}

/// A Core Foundation string, released when dropped.
struct Key(CFStringRef);

impl Key {
    fn new(text: &str) -> Option<Self> {
        let text = CString::new(text).ok()?;
        // SAFETY: a C string; the result is owned here.
        let key = unsafe {
            CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), CF_STRING_ENCODING_UTF8)
        };
        (!key.is_null()).then_some(Self(key))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: created in `new`.
        unsafe { CFRelease(self.0) };
    }
}

/// A number in a dictionary, if the key is there and holds one.
fn number(dictionary: CFDictionaryRef, key: &str) -> Option<i64> {
    let key = Key::new(key)?;
    // SAFETY: a dictionary and a key that are alive; the value is not owned.
    let value = unsafe { CFDictionaryGetValue(dictionary, key.0) };
    // SAFETY: a type check before reading the value as a number.
    if value.is_null() || unsafe { CFGetTypeID(value) != CFNumberGetTypeID() } {
        return None;
    }
    let mut read: i64 = 0;
    // SAFETY: as above.
    (unsafe { CFNumberGetValue(value, CF_NUMBER_SINT64, (&raw mut read).cast()) } != 0)
        .then_some(read)
}

/// The first accelerator's statistics: its busyness and memory.
fn statistics() -> Option<(Option<f32>, Option<GpuMemory>)> {
    let class = c"IOAccelerator";
    let mut iterator: IoObject = 0;
    // SAFETY: the matching dictionary is taken over by the call.
    let status = unsafe {
        IOServiceGetMatchingServices(0, IOServiceMatching(class.as_ptr()), &raw mut iterator)
    };
    if status != 0 {
        return None;
    }
    let mut result = None;
    loop {
        // SAFETY: an iterator the call gave.
        let service = unsafe { IOIteratorNext(iterator) };
        if service == 0 {
            break;
        }
        let mut properties: CFMutableDictionaryRef = std::ptr::null_mut();
        // SAFETY: a service of the iterator; the properties are owned here.
        let read = unsafe {
            IORegistryEntryCreateCFProperties(service, &raw mut properties, std::ptr::null(), 0)
        };
        if read == 0 && !properties.is_null() {
            let key = Key::new("PerformanceStatistics");
            // SAFETY: a dictionary that is alive.
            let performance = key.map(|key| unsafe { CFDictionaryGetValue(properties, key.0) });
            if let Some(performance) = performance
                && !performance.is_null()
                // SAFETY: a type check before using it as a dictionary.
                && unsafe { CFGetTypeID(performance) == CFDictionaryGetTypeID() }
            {
                let busy = number(performance, "Device Utilization %")
                    .map(|percent| (percent.clamp(0, 100) as f32) / 100.0);
                let used = number(performance, "In use system memory")
                    .or_else(|| number(performance, "vramUsedBytes"))
                    .and_then(|used| u64::try_from(used).ok());
                let total = number(properties, "VRAM,totalMB")
                    .and_then(|mib| u64::try_from(mib).ok())
                    .map(|mib| mib * 1024 * 1024);
                if busy.is_some() || used.is_some() {
                    result = Some((busy, used.map(|used| GpuMemory { used, total })));
                }
            }
            // SAFETY: created by the call above.
            unsafe { CFRelease(properties) };
        }
        // SAFETY: a service of the iterator.
        unsafe { IOObjectRelease(service) };
        if result.is_some() {
            break;
        }
    }
    // SAFETY: the iterator the call gave.
    unsafe { IOObjectRelease(iterator) };
    result
}

struct Accelerator;

impl GpuSource for Accelerator {
    fn read(&mut self) -> std::io::Result<(Option<f32>, Option<GpuMemory>)> {
        statistics().ok_or_else(|| failed("the graphics card"))
    }
}

pub fn open_gpu() -> Option<Box<dyn GpuSource>> {
    statistics().map(|_| Box::new(Accelerator) as Box<dyn GpuSource>)
}
