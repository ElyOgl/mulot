//! Samples the machine once per tick. Rates (network, disk I/O, per-process
//! I/O) are derived from the delta between two consecutive refreshes, so
//! nothing is ever shelled out and every panel works on macOS, Linux and
//! Windows.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde::Serialize;
use starship_battery::units::{power::watt, ratio::percent, time::second};
use sysinfo::{
    Components, Disk, DiskRefreshKind, Disks, Gpus, MINIMUM_CPU_UPDATE_INTERVAL, Networks, Pid,
    ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind, Users,
};

use crate::platform::{self, CoreTopology, MemBreakdown, Pressure};

/// Facts that do not change while mulot runs.
#[derive(Debug, Clone, Serialize)]
pub struct StaticInfo {
    pub host: String,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub cpu_model: String,
    pub logical_cores: usize,
    pub physical_cores: Option<usize>,
    pub topology: Option<CoreTopology>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Sample {
    pub uptime_secs: u64,
    pub load_average: [f64; 3],
    pub cpu: CpuSample,
    pub memory: MemSample,
    pub network: NetSample,
    pub disks: Vec<DiskSample>,
    pub disk_io: IoRate,
    pub gpu: Option<GpuSample>,
    pub cpu_temp_c: Option<f32>,
    pub cpu_temp_max_c: Option<f32>,
    pub sensors: Vec<SensorSample>,
    pub battery: Option<BatterySample>,
    pub process_count: usize,
    pub processes: Vec<ProcSample>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CpuSample {
    pub total: f64,
    pub per_core: Vec<f64>,
    pub frequency_mhz: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemSample {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub pressure: Option<Pressure>,
    pub breakdown: MemBreakdown,
}

impl MemSample {
    pub fn used_pct(&self) -> f64 {
        pct(self.used, self.total)
    }
    pub fn swap_pct(&self) -> f64 {
        pct(self.swap_used, self.swap_total)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct NetSample {
    pub rx_rate: f64,
    pub tx_rate: f64,
    pub rx_total: u64,
    pub tx_total: u64,
    pub interfaces: Vec<IfaceSample>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IfaceSample {
    pub name: String,
    pub rx_rate: f64,
    pub tx_rate: f64,
    pub rx_total: u64,
    pub tx_total: u64,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskSample {
    pub name: String,
    pub mount: String,
    pub file_system: String,
    pub total: u64,
    pub available: u64,
    pub removable: bool,
}

impl DiskSample {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
    pub fn used_pct(&self) -> f64 {
        pct(self.used(), self.total)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct IoRate {
    pub read: f64,
    pub write: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuSample {
    pub name: String,
    pub usage: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SensorSample {
    pub label: String,
    pub celsius: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatterySample {
    pub percent: f64,
    pub state: String,
    pub time_to_full_secs: Option<u64>,
    pub time_to_empty_secs: Option<u64>,
    pub health_percent: Option<f64>,
    pub cycles: Option<u32>,
    pub power_watts: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcSample {
    pub pid: u32,
    pub name: String,
    pub user: String,
    pub cpu: f64,
    pub memory: u64,
    pub read_rate: f64,
    pub write_rate: f64,
    pub status: String,
}

pub struct Collector {
    sys: System,
    networks: Networks,
    disks: Disks,
    components: Option<Components>,
    gpus: Option<Gpus>,
    users: Option<Users>,
    users_refreshed: Instant,
    user_names: HashMap<String, String>,
    battery: Option<(starship_battery::Manager, Vec<starship_battery::Battery>)>,
    last_battery: Option<BatterySample>,
    last: Instant,
    ticks: u64,
    info: StaticInfo,
}

fn sys_err(e: impl std::fmt::Debug) -> anyhow::Error {
    anyhow!("system query failed: {e:?}")
}

fn pct(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 * 100.0 / total as f64
    }
}

fn process_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_disk_usage()
        .with_user(UpdateKind::OnlyIfNotSet)
}

impl Collector {
    /// Builds the collector and primes every delta-based metric, so the
    /// first [`Collector::sample`] already reports real CPU usage and rates.
    pub fn new() -> Result<Self> {
        let mut sys = System::new().map_err(sys_err)?;
        sys.refresh_cpu_all();
        sys.refresh_memory();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, process_refresh_kind());

        let networks = Networks::new_with_refreshed_list().map_err(sys_err)?;
        let disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::everything())
            .map_err(sys_err)?;
        let components = Components::new_with_refreshed_list().ok();
        let gpus = Gpus::new_with_refreshed_list().ok();
        let users = Users::new_with_refreshed_list().ok();
        let battery = starship_battery::Manager::new().ok().and_then(|m| {
            let list: Vec<_> = m.batteries().ok()?.flatten().collect();
            (!list.is_empty()).then_some((m, list))
        });

        let cpus = sys.cpus();
        let info = StaticInfo {
            host: platform::computer_name().unwrap_or_else(|| {
                let host = System::host_name().unwrap_or_else(|_| "localhost".into());
                host.split('.').next().unwrap_or(&host).to_string()
            }),
            os: System::long_os_version()
                .or_else(|_| System::name())
                .unwrap_or_else(|_| std::env::consts::OS.into()),
            kernel: System::kernel_version().unwrap_or_default(),
            arch: System::cpu_arch(),
            cpu_model: cpus
                .first()
                .map(|c| c.brand().trim().to_string())
                .unwrap_or_default(),
            logical_cores: cpus.len(),
            physical_cores: System::physical_core_count().ok(),
            topology: platform::core_topology(),
        };

        let mut collector = Self {
            sys,
            networks,
            disks,
            components,
            gpus,
            users,
            users_refreshed: Instant::now(),
            user_names: HashMap::new(),
            battery,
            last_battery: None,
            last: Instant::now(),
            ticks: 0,
            info,
        };
        std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(250)));
        collector.last_battery = collector.read_battery();
        Ok(collector)
    }

    pub fn info(&self) -> &StaticInfo {
        &self.info
    }

    pub fn sample(&mut self) -> Sample {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64().max(0.001);
        self.last = now;
        self.ticks += 1;

        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();
        self.sys
            .refresh_processes_specifics(ProcessesToUpdate::All, true, process_refresh_kind());
        self.networks.refresh(true);
        self.disks
            .refresh_specifics(true, DiskRefreshKind::everything());
        if let Some(gpus) = self.gpus.as_mut() {
            gpus.refresh(false);
        }
        if let Some(components) = self.components.as_mut() {
            components.refresh(false);
        }
        if self.ticks.is_multiple_of(5) {
            self.last_battery = self.read_battery();
        }

        let load = System::load_average().ok();
        let (cpu_temp_c, cpu_temp_max_c, sensors) = self.read_sensors();
        let processes = self.read_processes(elapsed);

        Sample {
            uptime_secs: System::uptime().unwrap_or(0),
            load_average: load.map(|l| [l.one, l.five, l.fifteen]).unwrap_or_default(),
            cpu: self.read_cpu(),
            memory: self.read_memory(),
            network: self.read_network(elapsed),
            disks: self.read_disks(),
            disk_io: self.read_disk_io(elapsed),
            gpu: self.read_gpu(),
            cpu_temp_c,
            cpu_temp_max_c,
            sensors,
            battery: self.last_battery.clone(),
            process_count: processes.len(),
            processes,
        }
    }

    fn read_cpu(&self) -> CpuSample {
        let cpus = self.sys.cpus();
        CpuSample {
            total: f64::from(self.sys.global_cpu_usage()),
            per_core: cpus.iter().map(|c| f64::from(c.usage())).collect(),
            frequency_mhz: cpus.iter().map(|c| c.frequency()).max().unwrap_or(0),
        }
    }

    fn read_memory(&self) -> MemSample {
        MemSample {
            total: self.sys.total_memory(),
            used: self.sys.used_memory(),
            available: self.sys.available_memory(),
            swap_total: self.sys.total_swap(),
            swap_used: self.sys.used_swap(),
            pressure: platform::memory_pressure(),
            breakdown: platform::memory_breakdown().unwrap_or_default(),
        }
    }

    fn read_network(&self, elapsed: f64) -> NetSample {
        let mut net = NetSample::default();
        for (name, data) in &self.networks {
            let name = name.to_string_lossy();
            if is_virtual_interface(&name) {
                continue;
            }
            let iface = IfaceSample {
                name: name.into_owned(),
                rx_rate: data.received() as f64 / elapsed,
                tx_rate: data.transmitted() as f64 / elapsed,
                rx_total: data.total_received(),
                tx_total: data.total_transmitted(),
                addresses: {
                    let mut addrs: Vec<_> = data.ip_networks().iter().map(|n| n.addr).collect();
                    addrs.sort_by_key(|a| (a.is_ipv6(), a.is_loopback()));
                    addrs.iter().map(|a| a.to_string()).collect()
                },
            };
            net.rx_rate += iface.rx_rate;
            net.tx_rate += iface.tx_rate;
            net.rx_total += iface.rx_total;
            net.tx_total += iface.tx_total;
            if iface.rx_total + iface.tx_total > 0 {
                net.interfaces.push(iface);
            }
        }
        net.interfaces
            .sort_by_key(|i| std::cmp::Reverse(i.rx_total + i.tx_total));
        net
    }

    fn read_disks(&self) -> Vec<DiskSample> {
        let mut seen = HashSet::new();
        let mut disks: Vec<DiskSample> = self
            .disks
            .iter()
            .filter(|d| is_user_visible_disk(d))
            // APFS volumes in one container share their space numbers; keep
            // only the first (the list is ordered with `/` first).
            .filter(|d| seen.insert((d.total_space(), d.available_space())))
            .map(|d| DiskSample {
                name: d.name().to_string_lossy().into_owned(),
                mount: d.mount_point().to_string_lossy().into_owned(),
                file_system: d.file_system().to_string_lossy().into_owned(),
                total: d.total_space(),
                available: d.available_space(),
                removable: d.is_removable(),
            })
            .collect();
        disks.sort_by(|a, b| {
            (a.mount != "/")
                .cmp(&(b.mount != "/"))
                .then(a.mount.cmp(&b.mount))
        });
        disks
    }

    fn read_disk_io(&self, elapsed: f64) -> IoRate {
        // Volumes sharing one physical device report identical lifetime
        // counters; count each device once.
        let mut seen = HashSet::new();
        let mut io = IoRate::default();
        for disk in self.disks.iter() {
            let usage = disk.usage();
            let key = (usage.total_read_bytes, usage.total_written_bytes);
            if key == (0, 0) || !seen.insert(key) {
                continue;
            }
            io.read += usage.read_bytes as f64 / elapsed;
            io.write += usage.written_bytes as f64 / elapsed;
        }
        io
    }

    fn read_gpu(&self) -> Option<GpuSample> {
        let gpu = self.gpus.as_ref()?.iter().find(|g| g.usage().is_some())?;
        Some(GpuSample {
            name: gpu.model().or(gpu.vendor()).unwrap_or("GPU").to_string(),
            usage: f64::from(gpu.usage()?).clamp(0.0, 100.0),
        })
    }

    fn read_sensors(&self) -> (Option<f32>, Option<f32>, Vec<SensorSample>) {
        let Some(components) = self.components.as_ref() else {
            return (None, None, Vec::new());
        };
        let sensors: Vec<SensorSample> = components
            .iter()
            .filter_map(|c| {
                let celsius = c.temperature()?;
                (celsius.is_finite() && celsius > 0.0 && celsius < 150.0).then(|| SensorSample {
                    label: c.label().to_string(),
                    celsius,
                })
            })
            .collect();
        let cpu: Vec<f32> = sensors
            .iter()
            .filter(|s| is_cpu_sensor(&s.label))
            .map(|s| s.celsius)
            .collect();
        let pool: Vec<f32> = if cpu.is_empty() {
            sensors.iter().map(|s| s.celsius).collect()
        } else {
            cpu
        };
        if pool.is_empty() {
            return (None, None, sensors);
        }
        let avg = pool.iter().sum::<f32>() / pool.len() as f32;
        let max = pool.iter().copied().fold(f32::MIN, f32::max);
        (Some(avg), Some(max), sensors)
    }

    fn read_battery(&mut self) -> Option<BatterySample> {
        let (manager, batteries) = self.battery.as_mut()?;
        let battery = batteries.first_mut()?;
        manager.refresh(battery).ok()?;
        let secs = |t: Option<starship_battery::units::Time>| {
            t.map(|t| t.get::<second>())
                .filter(|s| s.is_finite() && *s > 0.0)
                .map(|s| s as u64)
        };
        let health = f64::from(battery.state_of_health().get::<percent>());
        let power = f64::from(battery.energy_rate().get::<watt>()).abs();
        Some(BatterySample {
            percent: f64::from(battery.state_of_charge().get::<percent>()).clamp(0.0, 100.0),
            state: battery.state().to_string().to_lowercase(),
            time_to_full_secs: secs(battery.time_to_full()),
            time_to_empty_secs: secs(battery.time_to_empty()),
            health_percent: (health > 0.0).then_some(health.min(100.0)),
            cycles: battery.cycle_count(),
            power_watts: (power > 0.01).then_some(power),
        })
    }

    fn read_processes(&mut self, elapsed: f64) -> Vec<ProcSample> {
        if self.users_refreshed.elapsed() > Duration::from_secs(60) {
            if let Some(users) = self.users.as_mut() {
                users.refresh();
            }
            self.user_names.clear();
            self.users_refreshed = Instant::now();
        }
        let mut out = Vec::with_capacity(self.sys.processes().len());
        for (pid, p) in self.sys.processes() {
            if p.thread_kind().is_some() {
                continue;
            }
            let user = match p.user_id() {
                Some(uid) => {
                    let key = uid.to_string();
                    match self.user_names.get(&key) {
                        Some(name) => name.clone(),
                        None => {
                            let name = self
                                .users
                                .as_ref()
                                .and_then(|u| u.get_user_by_id(uid))
                                .map(|u| u.name().to_string())
                                .unwrap_or_else(|| key.clone());
                            self.user_names.insert(key, name.clone());
                            name
                        }
                    }
                }
                None => String::new(),
            };
            let io = p.disk_usage();
            out.push(ProcSample {
                pid: pid.as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                user,
                cpu: f64::from(p.cpu_usage()),
                memory: p.memory(),
                read_rate: io.read_bytes as f64 / elapsed,
                write_rate: io.written_bytes as f64 / elapsed,
                status: p.status().to_string(),
            });
        }
        out
    }

    /// Sends SIGTERM (or SIGKILL when `force`) to `pid`.
    pub fn signal(&self, pid: u32, force: bool) -> Result<(), String> {
        let process = self
            .sys
            .process(Pid::from_u32(pid))
            .ok_or("process already exited")?;
        let signal = if force { Signal::Kill } else { Signal::Term };
        match process.kill_with(signal) {
            Some(true) => Ok(()),
            Some(false) => Err("permission denied".into()),
            None if force => Err("signal not supported on this OS".into()),
            None => process
                .kill()
                .then_some(())
                .ok_or_else(|| "permission denied".into()),
        }
    }
}

fn is_virtual_interface(name: &str) -> bool {
    const PREFIXES: [&str; 14] = [
        "lo", "utun", "awdl", "llw", "bridge", "gif", "stf", "anpi", "ap", "docker", "veth", "br-",
        "virbr", "vmnet",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

fn is_user_visible_disk(d: &Disk) -> bool {
    let mount = d.mount_point().to_string_lossy();
    let fs = d.file_system().to_string_lossy().to_lowercase();
    const HIDDEN_FS: [&str; 10] = [
        "tmpfs", "devtmpfs", "squashfs", "overlay", "devfs", "autofs", "nullfs", "ramfs",
        "efivarfs", "proc",
    ];
    const HIDDEN_MOUNTS: [&str; 8] = [
        "/System/Volumes/",
        "/private/var/vm",
        "/Library/Developer/CoreSimulator",
        "/snap/",
        "/run",
        "/dev",
        "/sys",
        "/proc",
    ];
    // Mounted installers and other read-only images always look 100% full;
    // `/` stays because on macOS it is the sealed (read-only) system volume.
    d.total_space() > 0
        && (!d.is_read_only() || mount == "/")
        && !HIDDEN_FS.contains(&fs.as_str())
        && !HIDDEN_MOUNTS.iter().any(|m| mount.starts_with(m))
}

fn is_cpu_sensor(label: &str) -> bool {
    let l = label.to_lowercase();
    [
        "cpu", "tdie", "tctl", "package", "core ", "coretemp", "k10temp", "soc",
    ]
    .iter()
    .any(|k| l.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_virtual_interfaces() {
        assert!(is_virtual_interface("lo0"));
        assert!(is_virtual_interface("utun3"));
        assert!(is_virtual_interface("awdl0"));
        assert!(!is_virtual_interface("en0"));
        assert!(!is_virtual_interface("eth0"));
        assert!(!is_virtual_interface("wlan0"));
    }

    #[test]
    fn recognises_cpu_sensors() {
        assert!(is_cpu_sensor("PMU tdie3"));
        assert!(is_cpu_sensor("coretemp Package id 0"));
        assert!(is_cpu_sensor("k10temp Tctl"));
        assert!(!is_cpu_sensor("Battery"));
    }

    #[test]
    fn live_sample_is_sane() {
        let mut c = Collector::new().expect("collector");
        let s = c.sample();
        assert!(c.info().logical_cores > 0);
        assert_eq!(s.cpu.per_core.len(), c.info().logical_cores);
        assert!((0.0..=100.0).contains(&s.cpu.total));
        assert!(s.memory.total > 0 && s.memory.used <= s.memory.total);
        assert!(s.process_count > 0);
        assert!(s.processes.iter().any(|p| p.pid == std::process::id()));
    }
}
