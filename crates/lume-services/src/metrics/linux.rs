//! Reading a Linux system: `/proc` for the processor, memory and network, and for the
//! graphics card NVIDIA's management library where it is installed, else what the
//! amdgpu driver puts in `/sys`. Other cards are reported unavailable: the Intel drivers
//! give their busyness per process only.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use super::proc::{parse_meminfo, parse_net_dev, parse_number, parse_stat};
use super::{Counters, CpuTimes, GpuMemory, GpuSource, System, Usage};

pub struct LinuxSystem;

fn read(path: &str) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}

fn unreadable() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, "not as expected")
}

/// Whether a network interface is hardware: virtual ones (bridges, containers' pairs,
/// tunnels) carry traffic the hardware ones carry as well, and would count it twice.
fn is_hardware(name: &str) -> bool {
    Path::new("/sys/class/net")
        .join(name)
        .join("device")
        .exists()
}

impl System for LinuxSystem {
    fn cpu(&mut self) -> std::io::Result<CpuTimes> {
        parse_stat(&read("/proc/stat")?).ok_or_else(unreadable)
    }

    fn memory(&mut self) -> std::io::Result<Usage> {
        parse_meminfo(&read("/proc/meminfo")?).ok_or_else(unreadable)
    }

    fn network(&mut self) -> std::io::Result<Counters> {
        parse_net_dev(&read("/proc/net/dev")?, is_hardware).ok_or_else(unreadable)
    }
}

/// Opens the graphics card: NVIDIA's library first, then an amdgpu card.
pub fn open_gpu() -> Option<Box<dyn GpuSource>> {
    if let Some(nvidia) = Nvml::open() {
        return Some(Box::new(nvidia));
    }
    amdgpu_card().map(|card| Box::new(Amdgpu { card }) as Box<dyn GpuSource>)
}

/// The first card the amdgpu driver says how busy it is.
fn amdgpu_card() -> Option<PathBuf> {
    let mut cards: Vec<PathBuf> = std::fs::read_dir("/sys/class/drm")
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("card") && !name.contains('-'))
        })
        .filter(|path| path.join("device/gpu_busy_percent").exists())
        .collect();
    cards.sort();
    cards.into_iter().next()
}

struct Amdgpu {
    card: PathBuf,
}

impl GpuSource for Amdgpu {
    fn read(&mut self) -> std::io::Result<(Option<f32>, Option<GpuMemory>)> {
        let number = |file: &str| -> Option<u64> {
            parse_number(&std::fs::read_to_string(self.card.join("device").join(file)).ok()?)
        };
        let busy = number("gpu_busy_percent").ok_or_else(unreadable)?;
        let memory = number("mem_info_vram_used").map(|used| GpuMemory {
            used,
            total: number("mem_info_vram_total"),
        });
        #[allow(clippy::cast_precision_loss, reason = "a percentage")]
        Ok((Some((busy.min(100) as f32) / 100.0), memory))
    }
}

/// What NVIDIA's management library calls its return codes and handles.
type NvmlReturn = i32;
type NvmlDevice = *mut c_void;

#[repr(C)]
#[derive(Default)]
struct NvmlUtilization {
    gpu: u32,
    memory: u32,
}

#[repr(C)]
#[derive(Default)]
struct NvmlMemory {
    total: u64,
    free: u64,
    used: u64,
}

/// NVIDIA's management library, opened. The library is the one the driver installs
/// on the system's own library path.
struct Nvml {
    // Kept so that the functions below stay loaded.
    _library: libloading::Library,
    shutdown: unsafe extern "C" fn() -> NvmlReturn,
    utilization: unsafe extern "C" fn(NvmlDevice, *mut NvmlUtilization) -> NvmlReturn,
    memory: unsafe extern "C" fn(NvmlDevice, *mut NvmlMemory) -> NvmlReturn,
    device: NvmlDevice,
}

// The handle is the library's, and the library may be called from any thread.
unsafe impl Send for Nvml {}

const NVML_SUCCESS: NvmlReturn = 0;

impl Nvml {
    fn open() -> Option<Self> {
        // SAFETY: loading the driver's own library by its versioned name; its start-up
        // code is the driver's.
        let library = unsafe { libloading::Library::new("libnvidia-ml.so.1") }.ok()?;
        // SAFETY: the signatures are those of NVIDIA's published header for these
        // versions of the functions.
        unsafe {
            let init = *library
                .get::<unsafe extern "C" fn() -> NvmlReturn>(b"nvmlInit_v2\0")
                .ok()?;
            let shutdown = *library
                .get::<unsafe extern "C" fn() -> NvmlReturn>(b"nvmlShutdown\0")
                .ok()?;
            let count = *library
                .get::<unsafe extern "C" fn(*mut u32) -> NvmlReturn>(b"nvmlDeviceGetCount_v2\0")
                .ok()?;
            let by_index = *library
                .get::<unsafe extern "C" fn(u32, *mut NvmlDevice) -> NvmlReturn>(
                    b"nvmlDeviceGetHandleByIndex_v2\0",
                )
                .ok()?;
            let utilization = *library
                .get::<unsafe extern "C" fn(NvmlDevice, *mut NvmlUtilization) -> NvmlReturn>(
                    b"nvmlDeviceGetUtilizationRates\0",
                )
                .ok()?;
            let memory = *library
                .get::<unsafe extern "C" fn(NvmlDevice, *mut NvmlMemory) -> NvmlReturn>(
                    b"nvmlDeviceGetMemoryInfo\0",
                )
                .ok()?;
            if init() != NVML_SUCCESS {
                return None;
            }
            let mut devices = 0;
            let mut device: NvmlDevice = std::ptr::null_mut();
            if count(&raw mut devices) != NVML_SUCCESS
                || devices == 0
                || by_index(0, &raw mut device) != NVML_SUCCESS
            {
                shutdown();
                return None;
            }
            Some(Self {
                _library: library,
                shutdown,
                utilization,
                memory,
                device,
            })
        }
    }
}

impl GpuSource for Nvml {
    fn read(&mut self) -> std::io::Result<(Option<f32>, Option<GpuMemory>)> {
        let mut rates = NvmlUtilization::default();
        let mut memory = NvmlMemory::default();
        // SAFETY: the device handle came from the library, which is still loaded and
        // started, and the structures are the ones the functions fill.
        let (busy, used) = unsafe {
            (
                (self.utilization)(self.device, &raw mut rates),
                (self.memory)(self.device, &raw mut memory),
            )
        };
        if busy != NVML_SUCCESS {
            return Err(unreadable());
        }
        #[allow(clippy::cast_precision_loss, reason = "a percentage")]
        let share = (rates.gpu.min(100) as f32) / 100.0;
        let memory = (used == NVML_SUCCESS).then_some(GpuMemory {
            used: memory.used,
            total: Some(memory.total),
        });
        Ok((Some(share), memory))
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        // SAFETY: started in `open`, stopped once here.
        unsafe {
            (self.shutdown)();
        }
    }
}
