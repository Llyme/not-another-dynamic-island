//! CPU / RAM utilization for the hub's stat rings. The `System` handle is kept
//! across calls on purpose: CPU usage is a delta between two refreshes, so a
//! fresh `System` per call would always report 0%.

use crate::pc::Pc;
use serde::Serialize;
use sysinfo::System;

#[derive(Serialize)]
pub struct SysStats {
    pub cpu_pct: f32,
    pub ram_pct: f32,
    pub ram_used_gb: f64,
    pub ram_total_gb: f64,
}

pub fn new_system() -> System {
    let mut sys = System::new();
    sys.refresh_cpu_usage(); // prime the sampler; the first real read comes later
    sys
}

/// How busy the CPU and the memory are now.
pub fn read(state: &Pc) -> SysStats {
    let mut guard = state.sys.lock().unwrap();
    let sys = guard.get_or_insert_with(new_system);
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    let total = sys.total_memory() as f64;
    let used = sys.used_memory() as f64;
    let gb = 1024.0 * 1024.0 * 1024.0;
    SysStats {
        cpu_pct: sys.global_cpu_usage(),
        ram_pct: if total > 0.0 { (used / total * 100.0) as f32 } else { 0.0 },
        ram_used_gb: used / gb,
        ram_total_gb: total / gb,
    }
}

/// The programs using the most of the CPU now (all of a program's processes together), the busiest first: each its name, its
/// program's path (for its picture), its share of the whole CPU (0..100) and its memory (GB). Read only while asked (an open panel).
pub fn top(state: &Pc, n: usize) -> serde_json::Value {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
    let mut guard = state.sys.lock().unwrap();
    let sys = guard.get_or_insert_with(new_system);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new().with_cpu().with_memory().with_exe(sysinfo::UpdateKind::OnlyIfNotSet));
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f32;
    let mut by: std::collections::HashMap<String, (f32, u64)> = std::collections::HashMap::new();
    for p in sys.processes().values() {
        let Some(exe) = p.exe().map(|e| e.to_string_lossy().to_string()).filter(|e| !e.is_empty()) else { continue };
        let e = by.entry(exe).or_default();
        e.0 += p.cpu_usage() / cores;
        e.1 += p.memory();
    }
    let me = std::env::current_exe().ok().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut list: Vec<(String, f32, u64)> = by.into_iter().filter(|(e, _)| e.to_lowercase() != me).map(|(e, (c, m))| (e, c, m)).collect();
    list.sort_by(|a, b| b.1.total_cmp(&a.1));
    let gb = 1024.0 * 1024.0 * 1024.0;
    serde_json::Value::Array(
        list.into_iter()
            .take(n)
            .map(|(exe, cpu, mem)| {
                let name = crate::sensors::exeinfo::lookup(&exe).name.filter(|n| !n.is_empty()).unwrap_or_else(|| {
                    let f = exe.rsplit(['\\', '/']).next().unwrap_or(&exe);
                    f.strip_suffix(".exe").unwrap_or(f).to_string()
                });
                serde_json::json!({ "name": name, "exe": exe, "cpu": (cpu * 10.0).round() / 10.0, "ram_gb": (mem as f64 / gb * 10.0).round() / 10.0 })
            })
            .collect(),
    )
}
