//! OS-specific extras that sysinfo does not expose: Apple Silicon P/E core
//! topology, kernel memory pressure and the Activity Monitor memory breakdown.

use serde::Serialize;

/// Performance / efficiency core split on hybrid CPUs (Apple Silicon).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct CoreTopology {
    pub performance: usize,
    pub efficiency: usize,
}

impl CoreTopology {
    /// Short label for logical CPU `index`, e.g. `E3` or `P0`. On Apple
    /// Silicon the efficiency cluster is enumerated first.
    pub fn label(&self, index: usize) -> String {
        if index < self.efficiency {
            format!("E{index}")
        } else {
            format!("P{}", index - self.efficiency)
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
pub enum Pressure {
    Normal,
    Warn,
    Critical,
}

impl Pressure {
    pub fn label(self) -> &'static str {
        match self {
            Pressure::Normal => "normal",
            Pressure::Warn => "warn",
            Pressure::Critical => "critical",
        }
    }
}

/// Where the RAM went, in bytes. Fields are `None` when the OS has no
/// equivalent concept (e.g. compressed memory on Linux without zswap stats).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MemBreakdown {
    pub app: Option<u64>,
    pub wired: Option<u64>,
    pub compressed: Option<u64>,
    pub cached: Option<u64>,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::ffi::CString;
    use std::sync::OnceLock;

    fn sysctl_int(name: &str) -> Option<i64> {
        let cname = CString::new(name).ok()?;
        let mut buf = [0u8; 8];
        let mut len = buf.len();
        // SAFETY: `buf` is a valid writable buffer of `len` bytes and the
        // name is a NUL-terminated C string.
        let rc = unsafe {
            libc::sysctlbyname(
                cname.as_ptr(),
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        match (rc, len) {
            (0, 4) => Some(i32::from_ne_bytes(buf[..4].try_into().ok()?) as i64),
            (0, 8) => Some(i64::from_ne_bytes(buf)),
            _ => None,
        }
    }

    pub fn core_topology() -> Option<CoreTopology> {
        if sysctl_int("hw.nperflevels")? < 2 {
            return None;
        }
        Some(CoreTopology {
            performance: sysctl_int("hw.perflevel0.logicalcpu")? as usize,
            efficiency: sysctl_int("hw.perflevel1.logicalcpu")? as usize,
        })
    }

    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCDynamicStoreCopyComputerName(
            store: *const libc::c_void,
            encoding: *mut u32,
        ) -> *const libc::c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(
            string: *const libc::c_void,
            buffer: *mut libc::c_char,
            size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(cf: *const libc::c_void);
    }

    /// The user-facing name from System Settings › General › About, which
    /// unlike the network hostname does not change with every DHCP lease.
    pub fn computer_name() -> Option<String> {
        const UTF8: u32 = 0x0800_0100;
        // SAFETY: a null store is documented as "use a temporary session";
        // the returned CFString is owned by us and released below.
        unsafe {
            let name = SCDynamicStoreCopyComputerName(std::ptr::null(), std::ptr::null_mut());
            if name.is_null() {
                return None;
            }
            let mut buf = [0 as libc::c_char; 256];
            let ok = CFStringGetCString(name, buf.as_mut_ptr(), buf.len() as isize, UTF8);
            CFRelease(name);
            if ok == 0 {
                return None;
            }
            let s = std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .trim()
                .to_string();
            (!s.is_empty()).then_some(s)
        }
    }

    pub fn memory_pressure() -> Option<Pressure> {
        // 1 = normal, 2 = warn, 4 = critical (kern.memorystatus_vm_pressure_level)
        match sysctl_int("kern.memorystatus_vm_pressure_level")? {
            1 => Some(Pressure::Normal),
            2 => Some(Pressure::Warn),
            4 => Some(Pressure::Critical),
            _ => None,
        }
    }

    #[allow(deprecated)]
    fn host_port() -> libc::mach_port_t {
        static PORT: OnceLock<libc::mach_port_t> = OnceLock::new();
        // SAFETY: mach_host_self has no preconditions; caching it avoids
        // leaking a send right on every refresh.
        *PORT.get_or_init(|| unsafe { libc::mach_host_self() })
    }

    #[allow(deprecated)]
    pub fn memory_breakdown() -> Option<MemBreakdown> {
        // SAFETY: vm_statistics64 is plain old data; host_statistics64 fills
        // at most `count` integers into it.
        let stats = unsafe {
            let mut stats: libc::vm_statistics64 = std::mem::zeroed();
            let mut count = libc::HOST_VM_INFO64_COUNT;
            let rc = libc::host_statistics64(
                host_port(),
                libc::HOST_VM_INFO64,
                (&mut stats as *mut libc::vm_statistics64).cast(),
                &mut count,
            );
            if rc != libc::KERN_SUCCESS {
                return None;
            }
            stats
        };
        let page = sysctl_int("hw.pagesize").unwrap_or(16_384) as u64;
        let pages = |n: u32| u64::from(n) * page;
        Some(MemBreakdown {
            app: Some(pages(
                stats
                    .internal_page_count
                    .saturating_sub(stats.purgeable_count),
            )),
            wired: Some(pages(stats.wire_count)),
            compressed: Some(pages(stats.compressor_page_count)),
            cached: Some(pages(stats.external_page_count) + pages(stats.purgeable_count)),
        })
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;

    pub fn core_topology() -> Option<CoreTopology> {
        None
    }

    pub fn computer_name() -> Option<String> {
        None
    }

    /// Maps PSI `some avg10` (share of time tasks stalled on memory) onto
    /// the same three levels macOS reports.
    pub fn memory_pressure() -> Option<Pressure> {
        let text = std::fs::read_to_string("/proc/pressure/memory").ok()?;
        let avg10: f64 = text
            .lines()
            .find(|l| l.starts_with("some"))?
            .split_whitespace()
            .find_map(|kv| kv.strip_prefix("avg10="))?
            .parse()
            .ok()?;
        Some(if avg10 >= 25.0 {
            Pressure::Critical
        } else if avg10 >= 5.0 {
            Pressure::Warn
        } else {
            Pressure::Normal
        })
    }

    pub fn memory_breakdown() -> Option<MemBreakdown> {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let field = |name: &str| -> Option<u64> {
            text.lines()
                .find(|l| l.starts_with(name) && l[name.len()..].starts_with(':'))?
                .split_whitespace()
                .nth(1)?
                .parse::<u64>()
                .ok()
                .map(|kb| kb * 1024)
        };
        let total = field("MemTotal")?;
        let free = field("MemFree")?;
        let cached = field("Cached").unwrap_or(0)
            + field("Buffers").unwrap_or(0)
            + field("SReclaimable").unwrap_or(0);
        Some(MemBreakdown {
            app: Some(total.saturating_sub(free).saturating_sub(cached)),
            wired: field("Unevictable"),
            compressed: None,
            cached: Some(cached),
        })
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    use super::*;

    pub fn core_topology() -> Option<CoreTopology> {
        None
    }

    pub fn computer_name() -> Option<String> {
        None
    }
    pub fn memory_pressure() -> Option<Pressure> {
        None
    }
    pub fn memory_breakdown() -> Option<MemBreakdown> {
        None
    }
}

pub use imp::{computer_name, core_topology, memory_breakdown, memory_pressure};

/// Local wall-clock time as `HH:MM:SS`.
#[cfg(unix)]
pub fn local_time() -> String {
    // SAFETY: localtime_r writes into the caller-provided `tm`.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return String::new();
        }
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

#[cfg(not(unix))]
pub fn local_time() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(
        "{:02}:{:02}:{:02} UTC",
        secs / 3600 % 24,
        secs / 60 % 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn efficiency_cores_are_labelled_first() {
        let t = CoreTopology {
            performance: 4,
            efficiency: 6,
        };
        assert_eq!(t.label(0), "E0");
        assert_eq!(t.label(5), "E5");
        assert_eq!(t.label(6), "P0");
        assert_eq!(t.label(9), "P3");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_probes_return_data() {
        assert!(memory_pressure().is_some());
        let b = memory_breakdown().expect("host_statistics64");
        assert!(b.wired.unwrap() > 0);
    }
}
