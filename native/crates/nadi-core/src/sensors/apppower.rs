//! Each app's share of what the battery gives, counted by NADI itself (Windows keeps its own figures where only an administrator may
//! read them). While the PC runs on its battery, every few seconds the watts the battery gives are split among the programs by how much
//! of the processor and of the graphics card each uses; what no program accounts for (the screen, Windows' own) is kept apart. Kept in
//! five-minute buckets for a day, in NADI's folder, so a restart keeps them.

use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Instant;

/// the bucket each share is kept in, and how long they are kept
const BUCKET_MS: u64 = 300_000;
const KEEP_MS: u64 = 24 * 3600 * 1000;
/// how much of what the battery gives the apps may have at most (the screen and the rest of the PC always take some)
const APPS_MAX: f64 = 0.85;
/// a busy processor's weight: at this share of the whole processor (in all its apps), the apps take APPS_MAX
const BUSY: f64 = 0.55;
/// the key of what no app accounts for
const SYSTEM: &str = "";

struct State {
    sys: sysinfo::System,
    gpu: Option<GpuPer>,
    /// each bucket (its start, ms): each program's watt-hours (its path; "" for the rest)
    buckets: BTreeMap<u64, HashMap<String, f64>>,
    last: Option<Instant>,
    saved: Option<Instant>,
    loaded: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn file() -> Option<std::path::PathBuf> {
    Some(crate::paths::data_dir()?.join("app-power.json"))
}

/// Look now (the host calls it as it reads the battery, every few seconds, while a plugin asks for it).
pub fn sample(battery: &Value) {
    let mut guard = STATE.lock().unwrap();
    let st = guard.get_or_insert_with(|| State { sys: crate::sensors::stats::new_system(), gpu: GpuPer::open(), buckets: BTreeMap::new(), last: None, saved: None, loaded: false });
    if !st.loaded {
        st.loaded = true;
        if let Some(v) = file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str::<BTreeMap<u64, HashMap<String, f64>>>(&t).ok()) {
            st.buckets = v;
        }
    }
    let now = Instant::now();
    let dt = st.last.map(|l| now.duration_since(l).as_secs_f64()).unwrap_or(0.0).min(30.0);
    st.last = Some(now);
    // (each program's share of the whole processor, and of the graphics card: all its processes together)
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
    st.sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new().with_cpu().with_exe(UpdateKind::OnlyIfNotSet));
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    let gpu = st.gpu.as_mut().and_then(|g| g.read()).unwrap_or_default();
    let mut weight: HashMap<String, f64> = HashMap::new();
    for (pid, p) in st.sys.processes() {
        let Some(exe) = p.exe().map(|e| e.to_string_lossy().to_lowercase()).filter(|e| !e.is_empty()) else { continue };
        let cpu = p.cpu_usage() as f64 / 100.0 / cores;
        let g = gpu.get(&pid.as_u32()).copied().unwrap_or(0.0) / 100.0;
        let w = cpu + 0.6 * g;
        if w > 0.0005 {
            *weight.entry(exe).or_default() += w;
        }
    }
    // (on its battery only: what it gives, split)
    let on_battery = battery["present"] == true && battery["plugged"] != true;
    let watts = battery["rate_w"].as_f64().map(f64::abs).unwrap_or(0.0);
    if !on_battery || watts <= 0.0 || dt <= 0.0 {
        return;
    }
    let wh = watts * dt / 3600.0;
    let total: f64 = weight.values().sum();
    let apps = (total / BUSY).min(1.0) * APPS_MAX;
    let start = crate::paths::now_ms() / BUCKET_MS * BUCKET_MS;
    let bucket = st.buckets.entry(start).or_default();
    if total > 0.0 {
        for (exe, w) in &weight {
            *bucket.entry(exe.clone()).or_default() += wh * apps * w / total;
        }
    }
    *bucket.entry(SYSTEM.to_string()).or_default() += wh * (1.0 - apps);
    let cut = crate::paths::now_ms().saturating_sub(KEEP_MS);
    st.buckets.retain(|t, _| *t >= cut);
    if st.saved.map_or(true, |t| t.elapsed().as_secs() >= 60) {
        st.saved = Some(now);
        if let Some(f) = file() {
            // (only what counts: tiny shares dropped from what is kept)
            let kept: BTreeMap<u64, HashMap<String, f64>> = st.buckets.iter().map(|(t, m)| (*t, m.iter().filter(|(_, v)| **v >= 0.0005).map(|(k, v)| (k.clone(), *v)).collect())).collect();
            let _ = std::fs::write(f, serde_json::to_string(&kept).unwrap_or_default());
        }
    }
}

/// What drew the battery over the last `hours`: the programs that drew the most (each its path, its name, watt-hours and share),
/// then what no app accounts for (`system`).
fn over(buckets: &BTreeMap<u64, HashMap<String, f64>>, hours: u64) -> Value {
    let cut = crate::paths::now_ms().saturating_sub(hours * 3600 * 1000);
    let mut sum: HashMap<&str, f64> = HashMap::new();
    for (_, m) in buckets.range(cut / BUCKET_MS * BUCKET_MS..) {
        for (k, v) in m {
            *sum.entry(k.as_str()).or_default() += v;
        }
    }
    let total: f64 = sum.values().sum();
    if total <= 0.0 {
        return json!([]);
    }
    let system = sum.remove(SYSTEM).unwrap_or(0.0);
    let mut list: Vec<(&str, f64)> = sum.into_iter().collect();
    list.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut out: Vec<Value> = list
        .into_iter()
        .take(6)
        .filter(|(_, v)| *v / total >= 0.01)
        .map(|(exe, v)| {
            let name = crate::sensors::exeinfo::lookup(exe).name.unwrap_or_else(|| std::path::Path::new(exe).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
            json!({ "program": exe, "name": name, "wh": (v * 100.0).round() / 100.0, "share": v / total })
        })
        .collect();
    out.push(json!({ "system": true, "name": "Screen and System", "wh": (system * 100.0).round() / 100.0, "share": system / total }));
    Value::Array(out)
}

/// The last three hours, and the last day.
pub fn summary() -> Value {
    let guard = STATE.lock().unwrap();
    let Some(st) = guard.as_ref() else { return json!({ "3h": [], "24h": [] }) };
    json!({ "3h": over(&st.buckets, 3), "24h": over(&st.buckets, 24) })
}

/// Each process's share of the graphics card (its busiest engine), from the "GPU Engine" counters, as Task Manager reads it.
struct GpuPer {
    query: isize,
    counter: isize,
    samples: u32,
}

impl GpuPer {
    fn open() -> Option<GpuPer> {
        use windows::core::{w, PCWSTR};
        use windows::Win32::System::Performance::*;
        unsafe {
            let mut query = 0isize;
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                return None;
            }
            let mut counter = 0isize;
            if PdhAddEnglishCounterW(query, w!("\\GPU Engine(*)\\Utilization Percentage"), 0, &mut counter) != 0 {
                let _ = PdhCloseQuery(query);
                return None;
            }
            PdhCollectQueryData(query);
            Some(GpuPer { query, counter, samples: 0 })
        }
    }

    fn read(&mut self) -> Option<HashMap<u32, f64>> {
        use windows::Win32::System::Performance::*;
        // (processes that began later are not in the query: made again now and then)
        if self.samples >= 20 {
            *self = GpuPer::open()?;
            return None;
        }
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            self.samples += 1;
            let (mut size, mut count) = (0u32, 0u32);
            PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
            if size == 0 {
                return None;
            }
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
                return None;
            }
            let mut out: HashMap<u32, f64> = HashMap::new();
            for i in 0..count as usize {
                let item = &*items.add(i);
                if item.FmtValue.CStatus != 0 {
                    continue;
                }
                let name = item.szName.to_string().unwrap_or_default();
                let Some(pid) = name.strip_prefix("pid_").and_then(|r| r.split('_').next()).and_then(|p| p.parse::<u32>().ok()) else { continue };
                let v = item.FmtValue.Anonymous.doubleValue;
                let e = out.entry(pid).or_default();
                *e = e.max(v);
            }
            Some(out)
        }
    }
}

impl Drop for GpuPer {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Performance::PdhCloseQuery(self.query);
        }
    }
}
