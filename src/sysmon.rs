use serde::Serialize;
use std::{
    sync::{Arc, RwLock},
    thread,
    time::{Duration, Instant},
};
use sysinfo::{Disks, Networks, ProcessesToUpdate, System};

#[derive(Serialize, Clone, Debug, Default)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub mem: u64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Snapshot {
    pub host: String,
    pub cpu: f32,
    pub cores: Vec<f32>,
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
    pub uptime: u64,
    pub procs: Vec<ProcInfo>,
}

pub struct SysMon {
    latest: Arc<RwLock<Snapshot>>,
}

impl SysMon {
    pub fn start() -> Arc<Self> {
        let latest = Arc::new(RwLock::new(Snapshot::default()));
        let writer = latest.clone();

        thread::spawn(move || {
            let mut system = System::new_all();
            let mut networks = Networks::new_with_refreshed_list();
            let mut disks = Disks::new_with_refreshed_list();
            let mut prev = Instant::now();
            let mut prev_rx: u64 = 0;
            let mut prev_tx: u64 = 0;
            let mut tick: u64 = 0;

            loop {
                thread::sleep(Duration::from_secs(2));
                tick += 1;

                system.refresh_cpu_usage();
                system.refresh_memory();
                system.refresh_processes(ProcessesToUpdate::All, true);
                networks.refresh(false);

                let now = Instant::now();
                let dt = now.duration_since(prev).as_secs_f64().max(0.1);
                prev = now;

                let rx_total: u64 = networks.list().values().map(|n| n.total_received()).sum();
                let tx_total: u64 = networks
                    .list()
                    .values()
                    .map(|n| n.total_transmitted())
                    .sum();
                let rx_bps = ((rx_total.saturating_sub(prev_rx)) as f64 / dt) as u64;
                let tx_bps = ((tx_total.saturating_sub(prev_tx)) as f64 / dt) as u64;
                prev_rx = rx_total;
                prev_tx = tx_total;

                if tick % 5 == 0 {
                    disks.refresh(true);
                }
                let disk_total: u64 = disks.list().iter().map(|d| d.total_space()).sum();
                let disk_avail: u64 = disks.list().iter().map(|d| d.available_space()).sum();

                let mut procs: Vec<ProcInfo> = system
                    .processes()
                    .iter()
                    .map(|(pid, p)| ProcInfo {
                        pid: pid.as_u32(),
                        name: p.name().to_string_lossy().to_string(),
                        cpu: p.cpu_usage(),
                        mem: p.memory(),
                    })
                    .collect();
                procs.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
                procs.truncate(8);

                let load = System::load_average();
                let snapshot = Snapshot {
                    host: System::host_name().unwrap_or_default(),
                    cpu: system.global_cpu_usage(),
                    cores: system.cpus().iter().map(|c| c.cpu_usage()).collect(),
                    load1: load.one,
                    load5: load.five,
                    load15: load.fifteen,
                    mem_used: system.used_memory(),
                    mem_total: system.total_memory(),
                    swap_used: system.used_swap(),
                    swap_total: system.total_swap(),
                    disk_used: disk_total.saturating_sub(disk_avail),
                    disk_total,
                    net_rx_bps: rx_bps,
                    net_tx_bps: tx_bps,
                    uptime: System::uptime(),
                    procs,
                };
                if let Ok(mut guard) = writer.write() {
                    *guard = snapshot;
                }
            }
        });

        Arc::new(Self { latest })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.latest.read().map(|s| s.clone()).unwrap_or_default()
    }
}
